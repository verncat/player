//! Track identification using AcoustID + rusty-chromaprint.

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use rusqlite::params;
use rusty_chromaprint::{Configuration, FingerprintCompressor, Fingerprinter};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::{Arc, Mutex};

use symphonia::core::audio::{AudioBufferRef, Signal};
use symphonia::core::codecs::DecoderOptions;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

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
}

// ── Progress event ───────────────────────────────────────────────────────────

#[derive(Clone, Serialize)]
struct IdentifyProgress {
    current: usize,
    total: usize,
    track_id: i64,
    track_name: Option<String>,
    status: String, // fingerprinting, looking_up, found, not_found, error, done
    message: Option<String>,
}

fn emit_progress(
    app: &tauri::AppHandle,
    current: usize,
    total: usize,
    track_id: i64,
    track_name: Option<&str>,
    status: &str,
    message: Option<&str>,
) {
    use tauri::Emitter;
    let _ = app.emit(
        "identify-progress",
        IdentifyProgress {
            current,
            total,
            track_id,
            track_name: track_name.map(|s| s.to_owned()),
            status: status.to_owned(),
            message: message.map(|s| s.to_owned()),
        },
    );
}

// ── Audio decoding to i16 ────────────────────────────────────────────────────

fn decode_to_i16(path: &Path) -> Result<(Vec<i16>, u32, u16), String> {
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());

    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }

    let probed = symphonia::default::get_probe()
        .format(
            &hint,
            mss,
            &FormatOptions::default(),
            &MetadataOptions::default(),
        )
        .map_err(|e| format!("probe: {e}"))?;

    let mut format = probed.format;
    let track = format.default_track().ok_or("no audio track")?;
    let track_id = track.id;
    let codec_params = track.codec_params.clone();
    let sample_rate = codec_params.sample_rate.unwrap_or(44100);
    let channels = codec_params.channels.map(|c| c.count()).unwrap_or(2) as u16;

    let mut decoder = symphonia::default::get_codecs()
        .make(&codec_params, &DecoderOptions::default())
        .map_err(|e| format!("decoder: {e}"))?;

    let mut samples: Vec<i16> = Vec::new();

    loop {
        let packet = match format.next_packet() {
            Ok(p) => p,
            Err(symphonia::core::errors::Error::IoError(ref e))
                if e.kind() == std::io::ErrorKind::UnexpectedEof =>
            {
                break;
            }
            Err(_) => break,
        };

        if packet.track_id() != track_id {
            continue;
        }

        match decoder.decode(&packet) {
            Ok(decoded) => {
                append_i16_samples(&mut samples, &decoded, channels as usize);
            }
            Err(_) => continue,
        }
    }

    Ok((samples, sample_rate, channels))
}

fn append_i16_samples(out: &mut Vec<i16>, buf: &AudioBufferRef, channels: usize) {
    let frames = buf.frames();
    match buf {
        AudioBufferRef::F32(b) => {
            for frame in 0..frames {
                for ch in 0..channels {
                    out.push((b.chan(ch)[frame].clamp(-1.0, 1.0) * 32767.0) as i16);
                }
            }
        }
        AudioBufferRef::S16(b) => {
            for frame in 0..frames {
                for ch in 0..channels {
                    out.push(b.chan(ch)[frame]);
                }
            }
        }
        AudioBufferRef::S32(b) => {
            for frame in 0..frames {
                for ch in 0..channels {
                    out.push((b.chan(ch)[frame] >> 16) as i16);
                }
            }
        }
        AudioBufferRef::U8(b) => {
            for frame in 0..frames {
                for ch in 0..channels {
                    out.push(((b.chan(ch)[frame] as i16) - 128) * 256);
                }
            }
        }
        AudioBufferRef::F64(b) => {
            for frame in 0..frames {
                for ch in 0..channels {
                    out.push((b.chan(ch)[frame].clamp(-1.0, 1.0) as f32 * 32767.0) as i16);
                }
            }
        }
        _ => {}
    }
}

// ── Chromaprint fingerprinting ───────────────────────────────────────────────

fn generate_fingerprint(
    samples: &[i16],
    sample_rate: u32,
    channels: u16,
) -> Result<(String, u32), String> {
    let config = Configuration::preset_test2();
    let mut printer = Fingerprinter::new(&config);
    printer
        .start(sample_rate, channels as u32)
        .map_err(|e| format!("{e:?}"))?;
    printer.consume(samples);
    printer.finish();

    let raw = printer.fingerprint();
    if raw.is_empty() {
        return Err("empty fingerprint".into());
    }

    let compressed = FingerprintCompressor::from(&config).compress(raw);
    let encoded = URL_SAFE_NO_PAD.encode(&compressed);
    let duration_secs = samples.len() as u32 / channels as u32 / sample_rate;

    Ok((encoded, duration_secs))
}

// ── AcoustID lookup ──────────────────────────────────────────────────────────

struct LookupResult {
    title: Option<String>,
    artist: Option<String>,
    album: Option<String>,
}

fn acoustid_lookup(fingerprint: &str, duration: u32) -> Result<Option<LookupResult>, String> {
    let client = reqwest::blocking::Client::new();
    let resp = client
        .post("https://api.acoustid.org/v2/lookup")
        .form(&[
            ("client", ACOUSTID_API_KEY),
            ("duration", &duration.to_string()),
            ("fingerprint", fingerprint),
            ("meta", "recordings+releasegroups+compress"),
        ])
        .send()
        .map_err(|e| format!("HTTP error: {e}"))?;

    let status = resp.status();
    let body = resp.text().map_err(|e| format!("read body: {e}"))?;

    if !status.is_success() {
        return Err(format!("HTTP {status}: {body}"));
    }

    let json: AcoustIdResponse =
        serde_json::from_str(&body).map_err(|e| format!("JSON parse: {e} — body: {body}"))?;

    if json.status != "ok" {
        return Err(format!("AcoustID status '{}' — body: {body}", json.status));
    }

    let results = json.results.unwrap_or_default();
    let best = results
        .into_iter()
        .filter(|r| r.score >= 0.5)
        .max_by(|a, b| {
            a.score
                .partial_cmp(&b.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

    let Some(result) = best else {
        return Ok(None);
    };

    let recordings = result.recordings.unwrap_or_default();
    let Some(rec) = recordings.into_iter().next() else {
        return Ok(None);
    };

    let artist = rec
        .artists
        .unwrap_or_default()
        .into_iter()
        .map(|a| a.name)
        .collect::<Vec<_>>()
        .join(", ");
    let album = rec
        .releasegroups
        .unwrap_or_default()
        .into_iter()
        .next()
        .map(|rg| rg.title);

    Ok(Some(LookupResult {
        title: rec.title,
        artist: if artist.is_empty() { None } else { Some(artist) },
        album,
    }))
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

    for (i, id) in ids.iter().enumerate() {
        let track_info: Option<(String, Option<String>, Option<String>)> = {
            let c = conn.lock().unwrap();
            c.query_row(
                "SELECT path, title, artist FROM tracks WHERE id = ?1",
                params![id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .ok()
        };

        let Some((rel, title, artist)) = track_info else {
            emit_progress(app, i + 1, total, *id, None, "error", Some("track not found in DB"));
            continue;
        };

        let display_name = format!(
            "{}",
            title.as_deref().unwrap_or(
                artist.as_deref().map_or(rel.as_str(), |a| a)
            )
        );
        let dn = display_name.as_str();

        let abs = data_dir.join(&rel);
        emit_progress(app, i + 1, total, *id, Some(dn), "fingerprinting",
            Some(&format!("Decoding {rel}")));

        let fp_result =
            decode_to_i16(&abs).and_then(|(s, sr, ch)| {
                let sample_count = s.len();
                let dur = sample_count as u32 / ch as u32 / sr;
                eprintln!("[identify] {rel}: decoded {sample_count} samples, {sr}Hz, {ch}ch, ~{dur}s");
                generate_fingerprint(&s, sr, ch)
            });

        let (fp, dur) = match fp_result {
            Ok(r) => r,
            Err(e) => {
                emit_progress(app, i + 1, total, *id, Some(dn), "error",
                    Some(&format!("Fingerprint failed: {e}")));
                continue;
            }
        };

        emit_progress(app, i + 1, total, *id, Some(dn), "looking_up",
            Some(&format!("AcoustID lookup (duration={dur}s, fp_len={})…", fp.len())));

        // Rate limit: max 3 req/s
        std::thread::sleep(std::time::Duration::from_millis(340));

        match acoustid_lookup(&fp, dur) {
            Ok(Some(result)) => {
                {
                    let c = conn.lock().unwrap();
                    c.execute(
                        "UPDATE tracks SET title = COALESCE(?1, title), \
                         artist = COALESCE(?2, artist), \
                         album = COALESCE(?3, album), \
                         manually_edited = 1 WHERE id = ?4",
                        params![result.title, result.artist, result.album, id],
                    )
                    .ok();
                }
                let msg = format!(
                    "{} – {}",
                    result.artist.as_deref().unwrap_or("?"),
                    result.title.as_deref().unwrap_or("?")
                );
                emit_progress(app, i + 1, total, *id, Some(dn), "found", Some(&msg));
            }
            Ok(None) => {
                emit_progress(app, i + 1, total, *id, Some(dn), "not_found",
                    Some(&format!("No match for {dn}")));
            }
            Err(e) => {
                eprintln!("[identify] AcoustID error for {rel}: {e}");
                emit_progress(app, i + 1, total, *id, Some(dn), "error", Some(&e));
            }
        }
    }

    emit_progress(app, total, total, 0, None, "done", None);

    // Notify library changed so the UI refreshes
    use tauri::Emitter;
    let _ = app.emit("library-changed", ());
}
