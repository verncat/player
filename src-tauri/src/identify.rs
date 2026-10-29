//! Track identification using AcoustID + rusty-chromaprint.

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use flate2::{write::GzEncoder, Compression};
use reqwest::{header, StatusCode};
use rusqlite::params;
use rusty_chromaprint::{Configuration, FingerprintCompressor, Fingerprinter};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use symphonia::core::audio::{SampleBuffer, SignalSpec};
use symphonia::core::codecs::DecoderOptions;
use symphonia::core::formats::{FormatOptions, SeekMode, SeekTo};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;
use symphonia::core::units::{Time, TimeBase};

/// Your AcoustID application API key.
/// Register at <https://acoustid.org/new-application> to get one.
const ACOUSTID_API_KEY: &str = "EygTPZIzmm";

// ── AcoustID response types ──────────────────────────────────────────────────

#[derive(Deserialize)]
struct AcoustIdResponse {
    status: String,
    results: Option<Vec<AcoustIdResult>>,
}

#[derive(Deserialize)]
struct AcoustIdResult {
    score: f64,
    recordings: Option<Vec<AcoustIdRecording>>,
}

#[derive(Deserialize)]
struct AcoustIdRecording {
    title: Option<String>,
    artists: Option<Vec<AcoustIdArtist>>,
    releasegroups: Option<Vec<AcoustIdReleaseGroup>>,
}

#[derive(Deserialize)]
struct AcoustIdArtist {
    name: String,
}

#[derive(Deserialize)]
struct AcoustIdReleaseGroup {
    title: String,
    id: Option<String>,
}

// ── Progress event ───────────────────────────────────────────────────────────

#[derive(Clone, Serialize)]
struct IdentifyProgress {
    /// Number of tracks that reached a terminal result, not the active track number.
    current: usize,
    total: usize,
    track_id: i64,
    track_name: Option<String>,
    status: String, // fingerprinting, looking_up, fetching_cover, saving, found, not_found, error, done
    message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    track: Option<crate::library::Track>,
}

impl IdentifyProgress {
    fn new(
        index: usize,
        total: usize,
        track_id: i64,
        track_name: Option<&str>,
        status: &str,
        message: Option<&str>,
    ) -> Self {
        let current = match status {
            "found" | "not_found" | "error" => index.saturating_add(1),
            "done" => total,
            _ => index,
        }
        .min(total);
        Self {
            current,
            total,
            track_id,
            track_name: track_name.map(str::to_owned),
            status: status.to_owned(),
            message: message.map(str::to_owned),
            track: None,
        }
    }
}

fn emit_progress(
    app: &tauri::AppHandle,
    index: usize,
    total: usize,
    track_id: i64,
    track_name: Option<&str>,
    status: &str,
    message: Option<&str>,
) {
    use tauri::Emitter;
    let _ = app.emit(
        "identify-progress",
        IdentifyProgress::new(index, total, track_id, track_name, status, message),
    );
}

// Match fpcalc's default window. Only the compact fingerprint leaves the device.
const FINGERPRINT_SECONDS: u64 = 120;

#[derive(Debug)]
struct Fingerprint {
    encoded: String,
    duration_secs: u32,
}

fn valid_duration(value: Option<f64>) -> Option<f64> {
    value.filter(|v| v.is_finite() && *v > 0.0 && *v <= u32::MAX as f64)
}

fn timestamp_secs(time_base: TimeBase, timestamp: u64) -> f64 {
    let time = time_base.calc_time(timestamp);
    time.seconds as f64 + time.frac
}

fn generate_fingerprint(
    path: &Path,
    library_duration: Option<f64>,
    start_secs: f64,
    end_secs: Option<f64>,
) -> Result<Fingerprint, String> {
    if !start_secs.is_finite()
        || start_secs < 0.0
        || end_secs.is_some_and(|end| !end.is_finite() || end <= start_secs)
    {
        return Err("Invalid track boundaries".into());
    }
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }
    let mut format = symphonia::default::get_probe()
        .format(
            &hint,
            mss,
            &FormatOptions {
                enable_gapless: true,
                ..Default::default()
            },
            &MetadataOptions::default(),
        )
        .map_err(|e| format!("probe: {e}"))?
        .format;
    let track = format.default_track().ok_or("No audio track")?;
    let track_id = track.id;
    let params = track.codec_params.clone();
    let mut time_base = params.time_base.or_else(|| {
        params
            .sample_rate
            .filter(|rate| *rate > 0)
            .map(|rate| TimeBase::new(1, rate))
    });
    let file_duration = params
        .n_frames
        .and_then(|frames| time_base.map(|base| timestamp_secs(base, frames)));
    // AcoustID requires the whole track's duration, not the 120-second fingerprint window.
    let duration = valid_duration(end_secs.map(|end| end - start_secs))
        .or_else(|| valid_duration(library_duration))
        .or_else(|| valid_duration(file_duration.map(|end| end - start_secs)));
    let clip_end = end_secs
        .or_else(|| duration.map(|duration| start_secs + duration))
        .unwrap_or(f64::INFINITY)
        .min(start_secs + FINGERPRINT_SECONDS as f64);
    let mut decoder = crate::audio_codecs::registry()
        .make(&params, &DecoderOptions::default())
        .map_err(|e| format!("decoder: {e}"))?;
    if start_secs > 0.0 {
        // Decode enough pre-roll for Opus, then trim all codecs at sample precision.
        let from = (start_secs - 0.080).max(0.0);
        format
            .seek(
                SeekMode::Coarse,
                SeekTo::Time {
                    time: Time::new(from as u64, from.fract()),
                    track_id: Some(track_id),
                },
            )
            .map_err(|e| format!("seek: {e}"))?;
        decoder.reset();
    }

    let config = Configuration::preset_test2();
    let mut printer = Fingerprinter::new(&config);
    let mut spec: Option<SignalSpec> = None;
    let mut pcm: Option<SampleBuffer<i16>> = None;
    let mut processed_frames = 0u64;
    let mut last_end = 0.0f64;
    loop {
        let packet = match format.next_packet() {
            Ok(packet) => packet,
            Err(symphonia::core::errors::Error::IoError(error))
                if error.kind() == std::io::ErrorKind::UnexpectedEof =>
            {
                break
            }
            Err(error) => return Err(format!("read audio: {error}")),
        };
        if packet.track_id() != track_id {
            continue;
        }
        let packet_start = time_base.map(|base| timestamp_secs(base, packet.ts()));
        if let Some(base) = time_base {
            last_end = last_end.max(timestamp_secs(
                base,
                packet.ts().saturating_add(packet.dur()),
            ));
        }
        if packet_start.is_some_and(|start| start >= clip_end) {
            if duration.is_some() || end_secs.is_some() {
                break;
            }
            // Unindexed streams may have no declared duration. Scan packet timestamps
            // without decoding more PCM or extending the fingerprint.
            continue;
        }
        let decoded = match decoder.decode(&packet) {
            Ok(decoded) => decoded,
            Err(symphonia::core::errors::Error::DecodeError(_)) => continue,
            Err(error) => return Err(format!("decode audio: {error}")),
        };
        let current = *decoded.spec();
        if let Some(previous) = spec {
            if current.rate != previous.rate || current.channels != previous.channels {
                return Err("Audio format changed during fingerprinting".into());
            }
        } else {
            printer
                .start(current.rate, current.channels.count() as u32)
                .map_err(|e| format!("Chromaprint: {e}"))?;
            spec = Some(current);
            time_base = time_base.or_else(|| Some(TimeBase::new(1, current.rate)));
        }
        let channels = current.channels.count();
        let frames = decoded.frames();
        let packet_start = packet_start.unwrap_or(processed_frames as f64 / current.rate as f64);
        let skip = ((start_secs - packet_start).max(0.0) * current.rate as f64).round() as usize;
        let end = ((clip_end - packet_start).max(0.0) * current.rate as f64).round() as usize;
        let start = skip.min(frames);
        let end = end.min(frames).max(start);
        if end == start {
            continue;
        }
        if pcm
            .as_ref()
            .is_none_or(|buffer| buffer.capacity() < frames * channels)
        {
            pcm = Some(SampleBuffer::new(decoded.capacity() as u64, current));
        }
        let buffer = pcm.as_mut().unwrap();
        buffer.copy_interleaved_ref(decoded);
        printer.consume(&buffer.samples()[start * channels..end * channels]);
        processed_frames += (end - start) as u64;
        if processed_frames >= FINGERPRINT_SECONDS * current.rate as u64 && duration.is_some() {
            break;
        }
    }
    let spec = spec.ok_or("No decodable audio samples")?;
    printer.finish();
    let raw = printer.fingerprint();
    if raw.is_empty() {
        return Err("Audio is too short to generate a fingerprint".into());
    }
    let duration = duration
        .or_else(|| valid_duration(Some(last_end - start_secs)))
        .or_else(|| valid_duration(Some(processed_frames as f64 / spec.rate as f64)))
        .ok_or("Cannot determine track duration")?;
    let compressed = FingerprintCompressor::from(&config).compress(raw);
    // Chromaprint uses URL-safe Base64 without padding (standard Base64 is not compatible).
    Ok(Fingerprint {
        encoded: URL_SAFE_NO_PAD.encode(compressed),
        duration_secs: (duration as u32).max(1),
    })
}

// ── AcoustID lookup ──────────────────────────────────────────────────────────

#[derive(Debug)]
struct LookupResult {
    title: Option<String>,
    artist: Option<String>,
    album: Option<String>,
    release_group_id: Option<String>,
}

#[derive(Debug, PartialEq)]
enum NoMatchReason {
    NoFingerprint,
    LowConfidence,
    MissingMetadata,
}
impl NoMatchReason {
    fn message(&self) -> &'static str {
        match self {
            Self::NoFingerprint => "No matching audio fingerprint in AcoustID.",
            Self::LowConfidence => "AcoustID returned matches below the confidence threshold.",
            Self::MissingMetadata => "Audio matched in AcoustID, but no linked title or artist is available from MusicBrainz.",
        }
    }
}
#[derive(Debug)]
enum LookupOutcome {
    Matched(LookupResult),
    Unmatched(NoMatchReason),
}

fn select_lookup_result(mut results: Vec<AcoustIdResult>) -> LookupOutcome {
    if results.is_empty() {
        return LookupOutcome::Unmatched(NoMatchReason::NoFingerprint);
    }
    results.retain(|result| result.score.is_finite() && result.score >= 0.5);
    if results.is_empty() {
        return LookupOutcome::Unmatched(NoMatchReason::LowConfidence);
    }
    results.sort_by(|a, b| b.score.total_cmp(&a.score));
    for result in results {
        // The highest scoring AcoustID may have no MusicBrainz links. Keep looking
        // through other reliable candidates and recordings before reporting failure.
        for recording in result.recordings.unwrap_or_default() {
            let title = recording.title.filter(|value| !value.trim().is_empty());
            let artist = recording
                .artists
                .unwrap_or_default()
                .into_iter()
                .map(|artist| artist.name)
                .filter(|name| !name.trim().is_empty())
                .collect::<Vec<_>>()
                .join(", ");
            let artist = (!artist.is_empty()).then_some(artist);
            if title.is_none() && artist.is_none() {
                continue;
            }
            let group = recording
                .releasegroups
                .unwrap_or_default()
                .into_iter()
                .find(|group| !group.title.trim().is_empty());
            let release_group_id = group
                .as_ref()
                .and_then(|group| group.id.clone())
                .filter(|id| valid_mbid(id));
            let album = group.map(|group| group.title);
            return LookupOutcome::Matched(LookupResult {
                title,
                artist,
                album,
                release_group_id,
            });
        }
    }
    LookupOutcome::Unmatched(NoMatchReason::MissingMetadata)
}

fn encode_lookup_request(fingerprint: &str, duration: u32) -> Result<(Vec<u8>, usize), String> {
    let duration_str = duration.to_string();
    let form_body = serde_urlencoded::to_string([
        ("client", ACOUSTID_API_KEY),
        ("duration", duration_str.as_str()),
        ("fingerprint", fingerprint),
        ("meta", "recordings releasegroups compress"),
        ("format", "json"),
    ])
    .map_err(|e| format!("encode request: {e}"))?;

    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder
        .write_all(form_body.as_bytes())
        .map_err(|e| format!("compress request: {e}"))?;
    let compressed_body = encoder
        .finish()
        .map_err(|e| format!("finalize request compression: {e}"))?;

    Ok((compressed_body, form_body.len()))
}

fn acoustid_lookup(fingerprint: &str, duration: u32) -> Result<LookupOutcome, String> {
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(20))
        .user_agent(format!("player/{}", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|e| format!("build client: {e}"))?;

    let (compressed_body, form_body_len) = encode_lookup_request(fingerprint, duration)?;
    let compressed_body_len = compressed_body.len();
    let fp_len = fingerprint.len();

    eprintln!(
        "[identify] AcoustID request prepared: duration={}s fp_len={} form_body={}B gzip_body={}B",
        duration, fp_len, form_body_len, compressed_body_len
    );

    let mut last_err = String::new();
    for attempt in 0..3 {
        if attempt > 0 {
            std::thread::sleep(std::time::Duration::from_secs(2u64.pow(attempt)));
        }

        let started_at = Instant::now();

        let resp = match client
            .post("https://api.acoustid.org/v2/lookup")
            .header(header::CONTENT_ENCODING, "gzip")
            .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
            .header(header::ACCEPT, "application/json")
            .body(compressed_body.clone())
            .send()
        {
            Ok(r) => r,
            Err(e) => {
                let elapsed_ms = started_at.elapsed().as_millis();
                last_err = format!("HTTP error: {e}");
                eprintln!(
                    "[identify] attempt {} transport failure after {}ms: {} (duration={}s fp_len={} form_body={}B gzip_body={}B)",
                    attempt + 1,
                    elapsed_ms,
                    last_err,
                    duration,
                    fp_len,
                    form_body_len,
                    compressed_body_len
                );
                continue;
            }
        };

        let status = resp.status();
        let body = resp.text().map_err(|e| format!("read body: {e}"))?;
        let elapsed_ms = started_at.elapsed().as_millis();

        eprintln!(
            "[identify] attempt {} response after {}ms: status={} body_len={}B (duration={}s fp_len={} form_body={}B gzip_body={}B)",
            attempt + 1,
            elapsed_ms,
            status,
            body.len(),
            duration,
            fp_len,
            form_body_len,
            compressed_body_len
        );

        if status.is_server_error() {
            let raw_err = format!("HTTP {status}: {body}");
            eprintln!("[identify] attempt {}: {raw_err}", attempt + 1);
            last_err = if matches!(
                status,
                StatusCode::SERVICE_UNAVAILABLE
                    | StatusCode::BAD_GATEWAY
                    | StatusCode::GATEWAY_TIMEOUT
            ) || body.contains("upstream connect error")
                || body.contains("connection termination")
            {
                "AcoustID is temporarily unavailable. The request was sent, but the service failed upstream. Please retry later."
                    .to_string()
            } else {
                raw_err
            };
            continue;
        }

        if !status.is_success() {
            return Err(format!("HTTP {status}: {body}"));
        }

        let json: AcoustIdResponse =
            serde_json::from_str(&body).map_err(|e| format!("JSON parse: {e} — body: {body}"))?;

        if json.status != "ok" {
            if body.contains("invalid client") {
                return Err(
                    "AcoustID rejected the configured client key. The key is invalid or expired."
                        .to_string(),
                );
            }
            return Err(format!("AcoustID status '{}' — body: {body}", json.status));
        }

        let results = json
            .results
            .ok_or("AcoustID response did not include results")?;
        eprintln!(
            "[identify] attempt {}: {} candidates; {} with linked recordings",
            attempt + 1,
            results.len(),
            results
                .iter()
                .filter(|result| result
                    .recordings
                    .as_ref()
                    .is_some_and(|items| !items.is_empty()))
                .count()
        );
        return Ok(select_lookup_result(results));
    }

    Err(last_err)
}

// ── Optional cover artwork ───────────────────────────────────────────────────

const MAX_COVER_BYTES: usize = 5 * 1024 * 1024;

fn valid_mbid(id: &str) -> bool {
    id.len() == 36
        && id.bytes().enumerate().all(|(index, byte)| {
            if [8, 13, 18, 23].contains(&index) {
                byte == b'-'
            } else {
                byte.is_ascii_hexdigit()
            }
        })
}

fn cover_url(release_group_id: &str) -> Result<String, String> {
    if !valid_mbid(release_group_id) {
        return Err("Invalid MusicBrainz release-group ID".into());
    }
    Ok(format!(
        "https://coverartarchive.org/release-group/{release_group_id}/front-500"
    ))
}

fn read_cover_bytes(
    reader: &mut impl Read,
    content_length: Option<u64>,
) -> Result<Vec<u8>, String> {
    if content_length.is_some_and(|length| length > MAX_COVER_BYTES as u64) {
        return Err("Cover image exceeds 5 MiB".into());
    }
    let mut bytes = Vec::new();
    reader
        .take(MAX_COVER_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    if bytes.len() > MAX_COVER_BYTES {
        return Err("Cover image exceeds 5 MiB".into());
    }
    Ok(bytes)
}

fn download_cover(release_group_id: &str) -> Result<Option<Vec<u8>>, String> {
    let url = cover_url(release_group_id)?;
    let client = reqwest::blocking::Client::builder()
        .https_only(true)
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(15))
        .redirect(reqwest::redirect::Policy::limited(5))
        .user_agent(format!(
            "player/{} (https://github.com/verncat/player)",
            env!("CARGO_PKG_VERSION")
        ))
        .build()
        .map_err(|error| error.to_string())?;
    let mut response = client.get(url).send().map_err(|error| error.to_string())?;
    if response.status() == StatusCode::NOT_FOUND {
        return Ok(None);
    }
    if !response.status().is_success() {
        return Err(format!(
            "Cover Art Archive returned HTTP {}",
            response.status()
        ));
    }
    let length = response.content_length();
    read_cover_bytes(&mut response, length).map(Some)
}

fn cached_cover(
    cache: &mut HashMap<String, Option<String>>,
    data_dir: &Path,
    release_group_id: &str,
    fetch: impl FnOnce() -> Result<Option<Vec<u8>>, String>,
) -> Option<String> {
    if !valid_mbid(release_group_id) {
        return None;
    }
    if let Some(cached) = cache.get(release_group_id) {
        return cached.clone();
    }
    let hash = match fetch() {
        Ok(Some(bytes))
            if bytes.len() <= MAX_COVER_BYTES && image::guess_format(&bytes).is_ok() =>
        {
            crate::covers::store(data_dir, &bytes)
        }
        Ok(_) => None,
        Err(error) => {
            tracing::info!(target: "player_lib::identify", %error, "cover unavailable; keeping identified metadata");
            None
        }
    };
    cache.insert(release_group_id.to_owned(), hash.clone());
    hash
}

// ── Tauri command ────────────────────────────────────────────────────────────

#[tauri::command]
pub fn identify_tracks(
    ids: Vec<i64>,
    app: tauri::AppHandle,
    lib: tauri::State<'_, crate::library::LibraryState>,
) -> Result<(), String> {
    let data_dir = lib.data_dir().to_path_buf();
    let conn = lib.conn();

    std::thread::spawn(move || {
        run_identify(&app, &conn, &data_dir, &ids);
    });

    Ok(())
}

fn run_identify(
    app: &tauri::AppHandle,
    conn: &Arc<Mutex<rusqlite::Connection>>,
    data_dir: &Path,
    ids: &[i64],
) {
    let total = ids.len();
    let mut cover_cache = HashMap::new();

    for (i, id) in ids.iter().enumerate() {
        let track_info = {
            let c = conn.lock().unwrap();
            c.query_row(
                "SELECT path, title, artist, duration_secs FROM tracks WHERE id = ?1",
                params![id],
                |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, Option<String>>(1)?,
                        r.get::<_, Option<String>>(2)?,
                        r.get::<_, Option<f64>>(3)?,
                    ))
                },
            )
            .ok()
            .and_then(|info| {
                crate::library::track_source_for_track(&c, data_dir, *id)
                    .ok()
                    .flatten()
                    .map(|source| (info, source))
            })
        };

        let Some(((rel, title, artist, duration_secs), source)) = track_info else {
            emit_progress(
                app,
                i,
                total,
                *id,
                None,
                "error",
                Some("track not found in DB"),
            );
            continue;
        };

        let display_name = format!(
            "{}",
            title
                .as_deref()
                .unwrap_or(artist.as_deref().map_or(rel.as_str(), |a| a))
        );
        let dn = display_name.as_str();

        emit_progress(
            app,
            i,
            total,
            *id,
            Some(dn),
            "fingerprinting",
            Some("Generating a local audio fingerprint (up to 120 seconds)…"),
        );

        let fp_result = match source {
            crate::library::TrackSource::File { path } => {
                generate_fingerprint(&path, duration_secs, 0.0, None)
            }
            crate::library::TrackSource::CueSegment {
                audio_path,
                start_secs,
                end_secs,
                duration_secs: segment_duration,
            } => generate_fingerprint(
                &audio_path,
                segment_duration.or(duration_secs),
                start_secs,
                end_secs,
            ),
            crate::library::TrackSource::SoulseekPreview { .. } => {
                Err("Only library tracks can be identified".into())
            }
        };

        let Fingerprint {
            encoded: fp,
            duration_secs: dur,
        } = match fp_result {
            Ok(r) => r,
            Err(e) => {
                emit_progress(
                    app,
                    i,
                    total,
                    *id,
                    Some(dn),
                    "error",
                    Some(&format!("Fingerprint failed: {e}")),
                );
                continue;
            }
        };

        emit_progress(
            app,
            i,
            total,
            *id,
            Some(dn),
            "looking_up",
            Some(&format!(
                "Looking up audio fingerprint ({} bytes)…",
                fp.len()
            )),
        );

        // Rate limit: max 3 req/s
        std::thread::sleep(std::time::Duration::from_millis(340));

        match acoustid_lookup(&fp, dur) {
            Ok(LookupOutcome::Matched(result)) => {
                let existing_cover: Option<String> = conn
                    .lock()
                    .unwrap()
                    .query_row(
                        "SELECT cover_hash FROM tracks WHERE id = ?1",
                        params![id],
                        |row| row.get(0),
                    )
                    .ok()
                    .flatten();
                let cover_hash = if existing_cover.as_deref().is_none_or(str::is_empty) {
                    result.release_group_id.as_deref().and_then(|group| {
                        emit_progress(
                            app,
                            i,
                            total,
                            *id,
                            Some(dn),
                            "fetching_cover",
                            Some("Fetching album artwork…"),
                        );
                        cached_cover(&mut cover_cache, data_dir, group, || download_cover(group))
                    })
                } else {
                    None
                };

                emit_progress(
                    app,
                    i,
                    total,
                    *id,
                    Some(dn),
                    "saving",
                    Some("Applying metadata…"),
                );
                let saved = {
                    let c = conn.lock().unwrap();
                    crate::library::apply_identified_metadata(
                        &c,
                        *id,
                        result.title.as_deref(),
                        result.artist.as_deref(),
                        result.album.as_deref(),
                        cover_hash.as_deref(),
                    )
                };
                let track = match saved {
                    Ok(track) => track,
                    Err(message) => {
                        emit_progress(app, i, total, *id, Some(dn), "error", Some(&message));
                        continue;
                    }
                };
                let msg = format!(
                    "{} – {}",
                    result.artist.as_deref().unwrap_or("?"),
                    result.title.as_deref().unwrap_or("?")
                );
                let mut event = IdentifyProgress::new(i, total, *id, Some(dn), "found", Some(&msg));
                event.track = Some(track);
                use tauri::Emitter;
                let _ = app.emit("identify-progress", event);
            }
            Ok(LookupOutcome::Unmatched(reason)) => {
                emit_progress(
                    app,
                    i,
                    total,
                    *id,
                    Some(dn),
                    "not_found",
                    Some(reason.message()),
                );
            }
            Err(e) => {
                eprintln!("[identify] AcoustID error for {rel}: {e}");
                emit_progress(app, i, total, *id, Some(dn), "error", Some(&e));
            }
        }
    }

    emit_progress(app, total, total, 0, None, "done", None);

    // Notify library changed so the UI refreshes
    use tauri::Emitter;
    let _ = app.emit("library-changed", ());
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Seek, SeekFrom};
    use std::path::PathBuf;

    #[test]
    fn progress_counts_tracks_only_after_their_terminal_result() {
        let events = [
            (0, "fingerprinting", 0),
            (0, "looking_up", 0),
            (0, "fetching_cover", 0),
            (0, "saving", 0),
            (0, "found", 1),
            (1, "fingerprinting", 1),
            (1, "looking_up", 1),
            (1, "saving", 1),
            (1, "not_found", 2),
            (2, "done", 2),
        ];
        for (index, status, completed) in events {
            let event = IdentifyProgress::new(index, 2, 42, Some("Track"), status, None);
            assert_eq!(event.current, completed, "progress during {status}");
        }
        assert_eq!(
            IdentifyProgress::new(0, 1, 42, None, "error", None).current,
            1
        );
    }

    fn lookup_response(value: serde_json::Value) -> LookupOutcome {
        select_lookup_result(
            serde_json::from_value::<AcoustIdResponse>(value)
                .unwrap()
                .results
                .unwrap(),
        )
    }
    #[test]
    fn lookup_distinguishes_absence_confidence_and_missing_musicbrainz_metadata() {
        for (results, expected) in [
            (serde_json::json!([]), NoMatchReason::NoFingerprint),
            (
                serde_json::json!([{"score": 0.49, "recordings": [{"title": "Wrong"}]}]),
                NoMatchReason::LowConfidence,
            ),
            (
                serde_json::json!([{"score": 0.99}]),
                NoMatchReason::MissingMetadata,
            ),
            (
                serde_json::json!([{"score": 0.99, "recordings": [{"title": " ", "artists": []}]}]),
                NoMatchReason::MissingMetadata,
            ),
        ] {
            let outcome = lookup_response(serde_json::json!({"status": "ok", "results": results}));
            assert!(matches!(outcome, LookupOutcome::Unmatched(reason) if reason == expected));
        }
    }
    #[test]
    fn lookup_skips_empty_recordings_and_unlinked_candidates_without_losing_valid_matches() {
        let outcome = lookup_response(serde_json::json!({"status": "ok", "results": [
            {"score": 1.0},
            {"score": 0.8, "recordings": [{}, {"title": "Song", "artists": [{"name": "Artist"}], "releasegroups": [{"title": "Album"}]}]},
            {"score": 0.7, "recordings": [{"title": "Other"}]}
        ]}));
        let LookupOutcome::Matched(result) = outcome else {
            panic!("lost usable metadata")
        };
        assert_eq!(result.title.as_deref(), Some("Song"));
        assert_eq!(result.artist.as_deref(), Some("Artist"));
        assert_eq!(result.album.as_deref(), Some("Album"));
    }

    #[test]
    fn artwork_uses_the_id_of_the_selected_album_without_guessing_another_release() {
        let id = "48140466-cff6-3222-bd55-63c27e43190d";
        let outcome = lookup_response(serde_json::json!({"status": "ok", "results": [{"score": 1,
            "recordings": [{"title": "Song", "releasegroups": [
                {"title": "Album", "id": id}, {"title": "Compilation", "id": "c31a5e2b-0bf8-32e0-8aeb-ef4ba9973932"}
            ]}]
        }]}));
        let LookupOutcome::Matched(result) = outcome else {
            panic!("missing metadata")
        };
        assert_eq!(result.album.as_deref(), Some("Album"));
        assert_eq!(result.release_group_id.as_deref(), Some(id));
        assert_eq!(
            cover_url(id).unwrap(),
            format!("https://coverartarchive.org/release-group/{id}/front-500")
        );
        for invalid in ["../secret", "https://example.org", "not-a-uuid"] {
            assert!(cover_url(invalid).is_err());
        }
    }

    #[test]
    fn cover_requests_are_bounded_even_without_a_content_length() {
        let mut oversized = std::io::Cursor::new(vec![0u8; MAX_COVER_BYTES + 1]);
        assert!(read_cover_bytes(&mut oversized, None)
            .unwrap_err()
            .contains("5 MiB"));
        let mut declared = std::io::Cursor::new(b"ignored");
        assert!(read_cover_bytes(&mut declared, Some(MAX_COVER_BYTES as u64 + 1)).is_err());
        assert_eq!(declared.position(), 0);
    }

    #[test]
    fn cover_downloads_are_cached_and_failures_remain_optional() {
        let root = std::env::temp_dir().join(format!(
            "player-identify-covers-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let mut cache = HashMap::new();
        let id = "48140466-cff6-3222-bd55-63c27e43190d";
        let mut png = std::io::Cursor::new(Vec::new());
        image::DynamicImage::new_rgb8(8, 8)
            .write_to(&mut png, image::ImageFormat::Png)
            .unwrap();
        let hash = cached_cover(&mut cache, &root, id, || Ok(Some(png.into_inner()))).unwrap();
        assert!(crate::covers::is_stored(&root, &hash));
        assert_eq!(
            cached_cover(&mut cache, &root, id, || panic!(
                "must reuse downloaded album cover"
            )),
            Some(hash)
        );
        let other = "c31a5e2b-0bf8-32e0-8aeb-ef4ba9973932";
        assert!(cached_cover(&mut cache, &root, other, || Err(
            "Service unavailable".into()
        ))
        .is_none());
        assert!(cached_cover(&mut cache, &root, other, || panic!(
            "must not retry failed artwork for every track"
        ))
        .is_none());
        let mut cache = HashMap::new();
        assert!(cached_cover(&mut cache, &root, id, || Ok(None)).is_none());
        assert!(cached_cover(&mut cache, &root, other, || Ok(Some(
            b"<html>not an image</html>".to_vec()
        )))
        .is_none());
        std::fs::remove_dir_all(root).unwrap();
    }

    struct TestAudio(PathBuf);
    impl Drop for TestAudio {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }
    fn wav(seconds: f64, rate: u32, channels: u16, bits: u16, format: u16) -> TestAudio {
        let path = std::env::temp_dir().join(format!(
            "player-identify-{}-{}.wav",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let data_len = (seconds * rate as f64) as u32 * channels as u32 * (bits / 8) as u32;
        let block = channels * (bits / 8);
        let mut file = std::fs::File::create(&path).unwrap();
        file.write_all(b"RIFF").unwrap();
        file.write_all(&(36 + data_len).to_le_bytes()).unwrap();
        file.write_all(b"WAVEfmt ").unwrap();
        file.write_all(&16u32.to_le_bytes()).unwrap();
        file.write_all(&format.to_le_bytes()).unwrap();
        file.write_all(&channels.to_le_bytes()).unwrap();
        file.write_all(&rate.to_le_bytes()).unwrap();
        file.write_all(&(rate * block as u32).to_le_bytes())
            .unwrap();
        file.write_all(&block.to_le_bytes()).unwrap();
        file.write_all(&bits.to_le_bytes()).unwrap();
        file.write_all(b"data").unwrap();
        file.write_all(&data_len.to_le_bytes()).unwrap();
        // Sparse zeroed PCM is valid silence and keeps the large-file regression cheap.
        file.set_len(44 + data_len as u64).unwrap();
        TestAudio(path)
    }
    fn write_tone(path: &Path, offset_seconds: u32, seconds: u32, rate: u32) {
        let mut file = std::fs::OpenOptions::new().write(true).open(path).unwrap();
        file.seek(SeekFrom::Start(
            44 + offset_seconds as u64 * rate as u64 * 2,
        ))
        .unwrap();
        let samples: Vec<u8> = (0..rate * seconds)
            .flat_map(|frame| {
                let t = frame as f64 / rate as f64;
                let value = (t * std::f64::consts::TAU * (220.0 + 25.0 * t)).sin();
                ((value * 20_000.0) as i16).to_le_bytes()
            })
            .collect();
        file.write_all(&samples).unwrap();
    }

    #[test]
    fn fingerprint_matches_official_chromaprint_for_a_synthetic_chirp() {
        let audio = wav(6.0, 11_025, 1, 16, 1);
        write_tone(&audio.0, 0, 6, 11_025);
        let actual = generate_fingerprint(&audio.0, None, 0.0, None).unwrap();
        // fpcalc 1.6.2, default algorithm, same generated 11025 Hz mono PCM.
        let reference = [
            4189956916, 3918371380, 3918117428, 3918109300, 3954805348, 3928590948, 4264135271,
            4264118630, 1982666854, 1982807138, 1714303090, 644755538, 632172658, 883831362,
            883831363, 346960451, 78460487, 11352647, 11483727, 49249103, 40696175, 40696191,
            40696283, 40694187, 309129643, 846000555, 594342315,
        ];
        let config = Configuration::preset_test2();
        let compressed = FingerprintCompressor::from(&config).compress(&reference);
        assert_eq!(actual.encoded, URL_SAFE_NO_PAD.encode(compressed));
    }

    #[test]
    fn large_files_use_a_bounded_fingerprint_and_the_whole_track_duration() {
        let audio = wav(180.0, 48_000, 2, 16, 1);
        assert!(audio.0.metadata().unwrap().len() > 15 * 1024 * 1024);
        let fp = generate_fingerprint(&audio.0, None, 0.0, None).unwrap();
        assert_eq!(fp.duration_secs, 180);
        assert!(fp.encoded.len() < 10_000);
        assert!(fp
            .encoded
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_'));
        let compressed = URL_SAFE_NO_PAD.decode(&fp.encoded).unwrap();
        assert_eq!(compressed[0], 1); // Chromaprint's default algorithm.
        let count = u32::from_be_bytes([0, compressed[1], compressed[2], compressed[3]]);
        // 120 seconds generate about 950 sub-fingerprints; the old full-file path exceeds 1400.
        assert!(
            (800..1000).contains(&count),
            "unexpected fingerprint window: {count}"
        );
        let (body, _) = encode_lookup_request(&fp.encoded, fp.duration_secs).unwrap();
        assert!(body.len() < 10_000);
    }

    #[test]
    fn integer_24_bit_and_float_pcm_are_converted_like_16_bit_pcm() {
        let mut fingerprints = Vec::new();
        for (bits, format) in [(16, 1), (24, 1), (32, 3)] {
            let audio = wav(6.0, 11_025, 1, bits, format);
            let fp = generate_fingerprint(&audio.0, None, 0.0, None).unwrap();
            assert_eq!(fp.duration_secs, 6);
            fingerprints.push(fp.encoded);
        }
        assert_eq!(fingerprints[0], fingerprints[1]);
        assert_eq!(fingerprints[0], fingerprints[2]);
    }

    #[test]
    fn cue_segment_matches_the_same_audio_as_a_standalone_track() {
        let rate = 11_025;
        let album = wav(15.0, rate, 1, 16, 1);
        let track = wav(6.0, rate, 1, 16, 1);
        write_tone(&album.0, 3, 6, rate);
        write_tone(&track.0, 0, 6, rate);
        let segment = generate_fingerprint(&album.0, Some(15.0), 3.0, Some(9.0)).unwrap();
        let standalone = generate_fingerprint(&track.0, None, 0.0, None).unwrap();
        assert_eq!(segment.duration_secs, 6);
        assert_eq!(segment.encoded, standalone.encoded);
    }

    #[test]
    fn identification_uses_the_bundled_opus_decoder() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/opus-stereo.opus");
        let fp = generate_fingerprint(&path, None, 0.0, None).unwrap();
        assert!(!fp.encoded.is_empty());
        assert_eq!(fp.duration_secs, 3);
    }

    #[test]
    fn short_audio_and_invalid_track_boundaries_fail_explicitly() {
        let audio = wav(0.1, 11_025, 1, 16, 1);
        assert!(generate_fingerprint(&audio.0, None, 0.0, None)
            .unwrap_err()
            .contains("too short"));
        for (start, end) in [(f64::NAN, None), (-1.0, None), (3.0, Some(2.0))] {
            assert!(generate_fingerprint(&audio.0, None, start, end)
                .unwrap_err()
                .contains("boundaries"));
        }
    }

    #[test]
    fn lookup_payload_is_only_a_compressed_fingerprint_form_not_an_audio_upload() {
        let fingerprint = "AQAB_test-fingerprint";
        let (body, form_len) = encode_lookup_request(fingerprint, 321).unwrap();
        assert_eq!(&body[..2], &[0x1f, 0x8b]);
        let mut form = String::new();
        flate2::read::GzDecoder::new(body.as_slice())
            .read_to_string(&mut form)
            .unwrap();
        assert_eq!(form.len(), form_len);
        let fields: std::collections::HashMap<String, String> =
            serde_urlencoded::from_str(&form).unwrap();
        assert_eq!(fields.len(), 5);
        assert_eq!(fields["duration"], "321");
        assert_eq!(fields["fingerprint"], fingerprint);
        assert_eq!(fields["client"], ACOUSTID_API_KEY);
        assert_eq!(fields["meta"], "recordings releasegroups compress");
        assert!(form.contains("meta=recordings+releasegroups+compress"));
        assert!(!form.contains("meta=recordings%2B"));
        assert_eq!(fields["format"], "json");
    }
}
