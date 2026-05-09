use rusqlite::{params, Connection};
use serde_json::json;
use std::collections::HashMap;
use std::f32::consts::PI;
use std::fs::{self, File};
use std::io::{BufWriter, Read, Write};
use std::path::Path;

type DemoResult<T> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

struct DemoTrack {
    rel_path: &'static str,
    title: &'static str,
    artist: &'static str,
    album: &'static str,
    track_number: i64,
    duration_secs: u32,
    year: i64,
    genre: &'static str,
    tags: &'static str,
    play_count: i64,
    liked: bool,
    date_added_days_ago: i64,
    rarity: &'static str,
    cover_mark: &'static str,
    cover_primary: &'static str,
    cover_secondary: &'static str,
    cover_accent: &'static str,
    audio_seed: u32,
}

struct DemoHistoryEntry {
    rel_path: &'static str,
    hours_ago: i64,
}

struct DemoPlaylist {
    name: &'static str,
    pinned: bool,
    created_days_ago: i64,
    pinned_days_ago: Option<i64>,
    track_paths: &'static [&'static str],
}

pub fn is_demo_mode_enabled() -> bool {
    std::env::args_os().any(|arg| arg == std::ffi::OsStr::new("--demo"))
}

pub fn prepare_demo_library(data_dir: &Path) -> DemoResult<()> {
    if data_dir.exists() {
        fs::remove_dir_all(data_dir)?;
    }
    fs::create_dir_all(data_dir)?;

    for track in demo_tracks() {
        let abs_path = data_dir.join(track.rel_path);
        if let Some(parent) = abs_path.parent() {
            fs::create_dir_all(parent)?;
        }
        write_demo_wav(&abs_path, track.duration_secs, track.audio_seed)?;
    }

    Ok(())
}

pub fn seed_demo_database(conn: &Connection, data_dir: &Path) -> DemoResult<()> {
    conn.execute_batch(
        "DELETE FROM play_history;
         DELETE FROM playlist_tracks;
         DELETE FROM playlists;
         DELETE FROM smart_playlists;
         DELETE FROM device_config;",
    )?;

    conn.execute(
        "INSERT INTO device_config (
            id,
            emoji,
            device_name,
            sync_enabled,
            soulseek_enabled,
            soulseek_username,
            soulseek_password
        ) VALUES (1, ?1, ?2, 0, 0, '', '')",
        params!["🎧", "Demo Device"],
    )?;

    let now = now_secs();
    let mut track_ids = HashMap::new();

    for track in demo_tracks() {
        let abs_path = data_dir.join(track.rel_path);
        let file_hash = hash_file(&abs_path)?;
        let cover_data = build_cover_svg(track).into_bytes();
        let date_added = now.saturating_sub(track.date_added_days_ago.saturating_mul(86_400));

        let rows = conn.execute(
            "UPDATE tracks
                SET title = ?1,
                    artist = ?2,
                    album = ?3,
                    track_number = ?4,
                    duration_secs = ?5,
                    cover_data = ?6,
                    cover_mime = 'image/svg+xml',
                    cover_source_path = NULL,
                    cover_source_mtime = 0,
                    file_hash = ?7,
                    rarity = ?8,
                    manually_edited = 1,
                    is_liked = ?9,
                    play_count = ?10,
                    year = ?11,
                    genre = ?12,
                    date_added = ?13,
                    tags = ?14,
                    is_duplicate = 0
              WHERE path = ?15",
            params![
                track.title,
                track.artist,
                track.album,
                track.track_number,
                track.duration_secs as f64,
                cover_data,
                file_hash,
                track.rarity,
                if track.liked { 1 } else { 0 },
                track.play_count,
                track.year,
                track.genre,
                date_added,
                track.tags,
                track.rel_path,
            ],
        )?;

        if rows != 1 {
            return Err(format!("Demo track was not indexed: {}", track.rel_path).into());
        }

        let id: i64 = conn.query_row(
            "SELECT id FROM tracks WHERE path = ?1",
            params![track.rel_path],
            |row| row.get(0),
        )?;
        track_ids.insert(track.rel_path, id);
    }

    for entry in demo_history() {
        let track_id = track_ids
            .get(entry.rel_path)
            .copied()
            .ok_or_else(|| format!("Missing demo history track: {}", entry.rel_path))?;
        let played_at = now.saturating_sub(entry.hours_ago.saturating_mul(3_600));
        conn.execute(
            "INSERT INTO play_history (track_id, played_at) VALUES (?1, ?2)",
            params![track_id, played_at],
        )?;
    }

    for playlist in demo_playlists() {
        let created_at = now.saturating_sub(playlist.created_days_ago.saturating_mul(86_400));
        let pinned_at = playlist
            .pinned_days_ago
            .map(|days| now.saturating_sub(days.saturating_mul(86_400)));

        conn.execute(
            "INSERT INTO playlists (name, created_at, pinned, pinned_at) VALUES (?1, ?2, ?3, ?4)",
            params![
                playlist.name,
                created_at,
                if playlist.pinned { 1 } else { 0 },
                pinned_at,
            ],
        )?;
        let playlist_id = conn.last_insert_rowid();

        for (position, path) in playlist.track_paths.iter().enumerate() {
            let track_id = track_ids
                .get(path)
                .copied()
                .ok_or_else(|| format!("Missing demo playlist track: {}", path))?;
            conn.execute(
                "INSERT INTO playlist_tracks (playlist_id, track_id, position) VALUES (?1, ?2, ?3)",
                params![playlist_id, track_id, position as i64],
            )?;
        }
    }

    for smart in demo_smart_playlists() {
        conn.execute(
            "INSERT INTO smart_playlists (id, name, match_mode, rules_json, pinned, pinned_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                smart.id,
                smart.name,
                smart.match_mode,
                smart.rules_json,
                if smart.pinned { 1 } else { 0 },
                smart.pinned_at,
                now,
            ],
        )?;
    }

    Ok(())
}

struct DemoSmartPlaylist {
    id: &'static str,
    name: &'static str,
    match_mode: &'static str,
    rules_json: String,
    pinned: bool,
    pinned_at: Option<i64>,
}

fn demo_tracks() -> &'static [DemoTrack] {
    &[
        DemoTrack {
            rel_path: "Nova Vale/Neon Coast/01 - Midnight Radio.wav",
            title: "Midnight Radio",
            artist: "Nova Vale",
            album: "Neon Coast",
            track_number: 1,
            duration_secs: 96,
            year: 2024,
            genre: "Synthwave",
            tags: "night drive, neon, skyline",
            play_count: 21,
            liked: true,
            date_added_days_ago: 18,
            rarity: "Epic",
            cover_mark: "NC",
            cover_primary: "#0d1b4c",
            cover_secondary: "#ef476f",
            cover_accent: "#ffd166",
            audio_seed: 11,
        },
        DemoTrack {
            rel_path: "Nova Vale/Neon Coast/02 - Glass Horizon.wav",
            title: "Glass Horizon",
            artist: "Nova Vale",
            album: "Neon Coast",
            track_number: 2,
            duration_secs: 104,
            year: 2024,
            genre: "Synthwave",
            tags: "sunset, analog, pulse",
            play_count: 14,
            liked: false,
            date_added_days_ago: 16,
            rarity: "Rare",
            cover_mark: "NC",
            cover_primary: "#0d1b4c",
            cover_secondary: "#ef476f",
            cover_accent: "#ffd166",
            audio_seed: 17,
        },
        DemoTrack {
            rel_path: "Nova Vale/Neon Coast/03 - City Halo.wav",
            title: "City Halo",
            artist: "Nova Vale",
            album: "Neon Coast",
            track_number: 3,
            duration_secs: 88,
            year: 2024,
            genre: "Electronic",
            tags: "lights, downtown, shimmer",
            play_count: 9,
            liked: true,
            date_added_days_ago: 15,
            rarity: "Legendary",
            cover_mark: "NC",
            cover_primary: "#0d1b4c",
            cover_secondary: "#ef476f",
            cover_accent: "#ffd166",
            audio_seed: 23,
        },
        DemoTrack {
            rel_path: "Satellite Hearts/Static Bloom/01 - Slow Arcade.wav",
            title: "Slow Arcade",
            artist: "Satellite Hearts",
            album: "Static Bloom",
            track_number: 1,
            duration_secs: 112,
            year: 2023,
            genre: "Dream Pop",
            tags: "soft focus, arcade, haze",
            play_count: 27,
            liked: true,
            date_added_days_ago: 34,
            rarity: "Mythic",
            cover_mark: "SB",
            cover_primary: "#132238",
            cover_secondary: "#56cfe1",
            cover_accent: "#80ffdb",
            audio_seed: 31,
        },
        DemoTrack {
            rel_path: "Satellite Hearts/Static Bloom/02 - Rain Circuit.wav",
            title: "Rain Circuit",
            artist: "Satellite Hearts",
            album: "Static Bloom",
            track_number: 2,
            duration_secs: 101,
            year: 2023,
            genre: "Dream Pop",
            tags: "rain, chrome, dusk",
            play_count: 18,
            liked: true,
            date_added_days_ago: 30,
            rarity: "Epic",
            cover_mark: "SB",
            cover_primary: "#132238",
            cover_secondary: "#56cfe1",
            cover_accent: "#80ffdb",
            audio_seed: 37,
        },
        DemoTrack {
            rel_path: "Satellite Hearts/Static Bloom/03 - Luma Exit.wav",
            title: "Luma Exit",
            artist: "Satellite Hearts",
            album: "Static Bloom",
            track_number: 3,
            duration_secs: 92,
            year: 2023,
            genre: "Indietronica",
            tags: "flares, tape, glow",
            play_count: 7,
            liked: false,
            date_added_days_ago: 29,
            rarity: "Uncommon",
            cover_mark: "SB",
            cover_primary: "#132238",
            cover_secondary: "#56cfe1",
            cover_accent: "#80ffdb",
            audio_seed: 41,
        },
        DemoTrack {
            rel_path: "Io Motel/Dawn Static/01 - Blue Exit.wav",
            title: "Blue Exit",
            artist: "Io Motel",
            album: "Dawn Static",
            track_number: 1,
            duration_secs: 118,
            year: 2025,
            genre: "Ambient",
            tags: "fog, terminal, ambient",
            play_count: 11,
            liked: false,
            date_added_days_ago: 8,
            rarity: "Rare",
            cover_mark: "DS",
            cover_primary: "#261447",
            cover_secondary: "#7b2cbf",
            cover_accent: "#c77dff",
            audio_seed: 53,
        },
        DemoTrack {
            rel_path: "Io Motel/Dawn Static/02 - Soft Errors.wav",
            title: "Soft Errors",
            artist: "Io Motel",
            album: "Dawn Static",
            track_number: 2,
            duration_secs: 109,
            year: 2025,
            genre: "Ambient",
            tags: "drift, low light, focus",
            play_count: 13,
            liked: true,
            date_added_days_ago: 6,
            rarity: "Legendary",
            cover_mark: "DS",
            cover_primary: "#261447",
            cover_secondary: "#7b2cbf",
            cover_accent: "#c77dff",
            audio_seed: 59,
        },
    ]
}

fn demo_history() -> &'static [DemoHistoryEntry] {
    &[
        DemoHistoryEntry {
            rel_path: "Nova Vale/Neon Coast/01 - Midnight Radio.wav",
            hours_ago: 1,
        },
        DemoHistoryEntry {
            rel_path: "Satellite Hearts/Static Bloom/02 - Rain Circuit.wav",
            hours_ago: 3,
        },
        DemoHistoryEntry {
            rel_path: "Io Motel/Dawn Static/02 - Soft Errors.wav",
            hours_ago: 6,
        },
        DemoHistoryEntry {
            rel_path: "Satellite Hearts/Static Bloom/01 - Slow Arcade.wav",
            hours_ago: 11,
        },
        DemoHistoryEntry {
            rel_path: "Nova Vale/Neon Coast/03 - City Halo.wav",
            hours_ago: 18,
        },
        DemoHistoryEntry {
            rel_path: "Io Motel/Dawn Static/01 - Blue Exit.wav",
            hours_ago: 27,
        },
        DemoHistoryEntry {
            rel_path: "Satellite Hearts/Static Bloom/01 - Slow Arcade.wav",
            hours_ago: 36,
        },
        DemoHistoryEntry {
            rel_path: "Nova Vale/Neon Coast/02 - Glass Horizon.wav",
            hours_ago: 48,
        },
        DemoHistoryEntry {
            rel_path: "Io Motel/Dawn Static/02 - Soft Errors.wav",
            hours_ago: 61,
        },
        DemoHistoryEntry {
            rel_path: "Satellite Hearts/Static Bloom/03 - Luma Exit.wav",
            hours_ago: 74,
        },
        DemoHistoryEntry {
            rel_path: "Nova Vale/Neon Coast/01 - Midnight Radio.wav",
            hours_ago: 93,
        },
        DemoHistoryEntry {
            rel_path: "Satellite Hearts/Static Bloom/02 - Rain Circuit.wav",
            hours_ago: 117,
        },
    ]
}

fn demo_playlists() -> &'static [DemoPlaylist] {
    &[
        DemoPlaylist {
            name: "Night Drive",
            pinned: true,
            created_days_ago: 40,
            pinned_days_ago: Some(5),
            track_paths: &[
                "Nova Vale/Neon Coast/01 - Midnight Radio.wav",
                "Nova Vale/Neon Coast/02 - Glass Horizon.wav",
                "Nova Vale/Neon Coast/03 - City Halo.wav",
                "Satellite Hearts/Static Bloom/02 - Rain Circuit.wav",
            ],
        },
        DemoPlaylist {
            name: "Focus Grid",
            pinned: true,
            created_days_ago: 22,
            pinned_days_ago: Some(3),
            track_paths: &[
                "Io Motel/Dawn Static/01 - Blue Exit.wav",
                "Io Motel/Dawn Static/02 - Soft Errors.wav",
                "Satellite Hearts/Static Bloom/01 - Slow Arcade.wav",
                "Satellite Hearts/Static Bloom/03 - Luma Exit.wav",
            ],
        },
        DemoPlaylist {
            name: "Rain Window",
            pinned: false,
            created_days_ago: 12,
            pinned_days_ago: None,
            track_paths: &[
                "Satellite Hearts/Static Bloom/01 - Slow Arcade.wav",
                "Satellite Hearts/Static Bloom/02 - Rain Circuit.wav",
                "Io Motel/Dawn Static/02 - Soft Errors.wav",
            ],
        },
    ]
}

fn demo_smart_playlists() -> Vec<DemoSmartPlaylist> {
    vec![
        DemoSmartPlaylist {
            id: "demo-favorites",
            name: "Favorites",
            match_mode: "all",
            rules_json: json!([
                { "id": "liked", "field": "is_liked", "op": "is_true", "value": "" },
                { "id": "sort", "field": "sort", "op": "sort_desc", "value": "play_count" }
            ])
            .to_string(),
            pinned: true,
            pinned_at: Some(now_secs().saturating_sub(2 * 86_400)),
        },
        DemoSmartPlaylist {
            id: "demo-high-rotation",
            name: "High Rotation",
            match_mode: "all",
            rules_json: json!([
                { "id": "plays", "field": "play_count", "op": "gte", "value": "12" },
                { "id": "sort", "field": "sort", "op": "sort_desc", "value": "play_count" }
            ])
            .to_string(),
            pinned: false,
            pinned_at: None,
        },
        DemoSmartPlaylist {
            id: "demo-soft-focus",
            name: "Soft Focus",
            match_mode: "any",
            rules_json: json!([
                { "id": "genre", "field": "genre", "op": "in", "value": "[\"Ambient\",\"Dream Pop\"]" },
                { "id": "tags", "field": "tags", "op": "in", "value": "[\"focus\",\"rain\",\"haze\"]" },
                { "id": "sort", "field": "sort", "op": "sort_asc", "value": "artist" }
            ])
            .to_string(),
            pinned: false,
            pinned_at: None,
        },
    ]
}

fn build_cover_svg(track: &DemoTrack) -> String {
    format!(
                r##"<svg xmlns="http://www.w3.org/2000/svg" width="640" height="640" viewBox="0 0 640 640">
  <defs>
    <linearGradient id="bg" x1="0" y1="0" x2="1" y2="1">
      <stop offset="0%" stop-color="{primary}"/>
      <stop offset="100%" stop-color="{secondary}"/>
    </linearGradient>
  </defs>
  <rect width="640" height="640" rx="44" fill="url(#bg)"/>
  <circle cx="510" cy="138" r="110" fill="{accent}" fill-opacity="0.22"/>
  <circle cx="142" cy="520" r="150" fill="#ffffff" fill-opacity="0.08"/>
  <path d="M92 164 C192 86, 332 82, 502 176" stroke="#ffffff" stroke-opacity="0.18" stroke-width="22" fill="none"/>
  <path d="M120 438 C246 324, 412 304, 554 390" stroke="#ffffff" stroke-opacity="0.14" stroke-width="18" fill="none"/>
  <rect x="78" y="78" width="484" height="484" rx="34" fill="#0b1020" fill-opacity="0.10" stroke="#ffffff" stroke-opacity="0.10"/>
  <text x="88" y="518" fill="#ffffff" font-size="168" font-family="Avenir Next, Helvetica Neue, Arial, sans-serif" font-weight="800">{mark}</text>
  <text x="92" y="574" fill="#ffffff" fill-opacity="0.84" font-size="34" font-family="Avenir Next, Helvetica Neue, Arial, sans-serif">{album}</text>
</svg>"##,
        primary = track.cover_primary,
        secondary = track.cover_secondary,
        accent = track.cover_accent,
        mark = track.cover_mark,
        album = escape_svg_text(track.album),
    )
}

fn escape_svg_text(input: &str) -> String {
    input
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn write_demo_wav(path: &Path, duration_secs: u32, seed: u32) -> DemoResult<()> {
    const SAMPLE_RATE: u32 = 16_000;
    const CHANNELS: u16 = 1;
    const BITS_PER_SAMPLE: u16 = 16;

    let sample_count = duration_secs.saturating_mul(SAMPLE_RATE);
    let bytes_per_sample = (BITS_PER_SAMPLE / 8) as u32;
    let data_len = sample_count
        .saturating_mul(CHANNELS as u32)
        .saturating_mul(bytes_per_sample);
    let riff_len = 36u32.saturating_add(data_len);
    let byte_rate = SAMPLE_RATE * CHANNELS as u32 * bytes_per_sample;
    let block_align = CHANNELS * (BITS_PER_SAMPLE / 8);

    let file = File::create(path)?;
    let mut writer = BufWriter::new(file);

    writer.write_all(b"RIFF")?;
    writer.write_all(&riff_len.to_le_bytes())?;
    writer.write_all(b"WAVE")?;
    writer.write_all(b"fmt ")?;
    writer.write_all(&16u32.to_le_bytes())?;
    writer.write_all(&1u16.to_le_bytes())?;
    writer.write_all(&CHANNELS.to_le_bytes())?;
    writer.write_all(&SAMPLE_RATE.to_le_bytes())?;
    writer.write_all(&byte_rate.to_le_bytes())?;
    writer.write_all(&block_align.to_le_bytes())?;
    writer.write_all(&BITS_PER_SAMPLE.to_le_bytes())?;
    writer.write_all(b"data")?;
    writer.write_all(&data_len.to_le_bytes())?;

    for frame in 0..sample_count {
        let sample = synth_sample(seed, frame, SAMPLE_RATE);
        writer.write_all(&sample.to_le_bytes())?;
    }

    writer.flush()?;
    Ok(())
}

fn synth_sample(seed: u32, frame: u32, sample_rate: u32) -> i16 {
    let time = frame as f32 / sample_rate as f32;
    let bpm = 88.0 + (seed % 29) as f32;
    let beat_phase = (time * bpm / 60.0).fract();
    let beat = (1.0 - ((beat_phase - 0.08).abs() / 0.08)).clamp(0.0, 1.0);
    let pulse = beat.powf(2.8);

    let root = 150.0 + (seed % 11) as f32 * 14.0;
    let pad = (2.0 * PI * root * time).sin() * 0.26
        + (2.0 * PI * root * 1.25 * time).sin() * 0.18
        + (2.0 * PI * root * 1.5 * time).sin() * 0.12;
    let bass = (2.0 * PI * (root / 2.0) * time).sin() * (0.22 + pulse * 0.18);
    let shimmer_gate = ((time * 0.35 + (seed as f32 * 0.01)).sin() * 0.5 + 0.5).powf(2.0);
    let shimmer = (2.0 * PI * root * 2.04 * time).sin() * 0.08 * shimmer_gate;
    let signal = (pad * (0.42 + pulse * 0.22) + bass + shimmer).clamp(-0.9, 0.9);

    (signal * i16::MAX as f32 * 0.48) as i16
}

fn hash_file(path: &Path) -> DemoResult<String> {
    let mut file = File::open(path)?;
    let mut hasher = blake3::Hasher::new();
    let mut buf = [0u8; 8 * 1024];
    loop {
        let read = file.read(&mut buf)?;
        if read == 0 {
            break;
        }
        hasher.update(&buf[..read]);
    }
    Ok(hasher.finalize().to_hex().to_string())
}

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}