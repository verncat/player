//! Music library subsystem.
//!
//! Expected layout under `data_dir/`:
//!
//! ```text
//! data/
//!   Artist/
//!     Album/
//!       01 - Song.opus
//!     standalone.opus
//!   root-song.mp3
//!   app.db          ← created / managed here
//! ```
//!
//! Metadata priority:
//!   1. Tags embedded in the audio file (via lofty).
//!   2. Folder structure: first component → artist, second → album.
//!   3. Filename pattern: `<track>[-. ]<title>` or plain stem.

use base64::{engine::general_purpose::STANDARD as B64, Engine};
use lofty::picture::PictureType;
use lofty::prelude::{Accessor, AudioFile, TaggedFileExt};
use lofty::probe::Probe;
use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use rusqlite::{params, Connection};
use serde::Serialize;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use walkdir::WalkDir;

type BoxError = Box<dyn std::error::Error + Send + Sync>;

const AUDIO_EXTENSIONS: &[&str] = &[
    "mp3", "flac", "ogg", "opus", "aac", "m4a", "wav", "wv", "ape",
];

// ── Public types ──────────────────────────────────────────────────────────────

#[derive(Debug, Serialize, Clone)]
pub struct Track {
    pub id: i64,
    /// Path relative to `data_dir`, normalised to forward-slashes.
    pub path: String,
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub track_number: Option<i64>,
    pub duration_secs: Option<f64>,
    pub file_hash: Option<String>,
    pub rarity: Option<String>,
    pub manually_edited: bool,
}

// ── Internal types ────────────────────────────────────────────────────────────

#[derive(Default)]
struct Meta {
    title: Option<String>,
    artist: Option<String>,
    album: Option<String>,
    track_number: Option<i64>,
    duration_secs: Option<f64>,
    /// Raw bytes of the embedded cover image.
    cover_data: Option<Vec<u8>>,
    cover_mime: Option<String>,
}

// ── Managed state ─────────────────────────────────────────────────────────────

/// Tauri managed state for the music library. `Send + Sync` — safe to share.
pub struct LibraryState {
    conn: Arc<Mutex<Connection>>,
    pub data_dir: PathBuf,
}

impl LibraryState {
    /// Sets up the library subsystem:
    ///   1. Creates `data_dir` if missing.
    ///   2. Opens / migrates `data_dir/app.db`.
    ///   3. Full-scans `data_dir` on startup.
    ///   4. Spawns a background FS watcher for incremental updates.
    pub fn new(data_dir: PathBuf, app_handle: tauri::AppHandle) -> Result<Self, BoxError> {
        std::fs::create_dir_all(&data_dir)?;

        let conn = Connection::open(data_dir.join("app.db"))?;
        init_schema(&conn)?;
        index_directory(&conn, &data_dir)?;

        let conn = Arc::new(Mutex::new(conn));
        start_watcher(data_dir.clone(), Arc::clone(&conn), app_handle)?;

        Ok(Self { conn, data_dir })
    }

    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    pub fn conn(&self) -> Arc<Mutex<Connection>> {
        Arc::clone(&self.conn)
    }

    pub fn search(&self, query: &str) -> Result<Vec<Track>, BoxError> {
        let conn = self.conn.lock().unwrap();
        let pat = format!("%{query}%");
        let mut stmt = conn.prepare(
            "SELECT id, path, title, artist, album, track_number, duration_secs, file_hash, rarity, manually_edited
               FROM tracks
              WHERE title  LIKE ?1 COLLATE NOCASE
                 OR artist LIKE ?1 COLLATE NOCASE
                 OR album  LIKE ?1 COLLATE NOCASE
              ORDER BY artist, album, track_number, title",
        )?;
        let tracks = stmt
            .query_map(params![pat], row_to_track)?
            .filter_map(|r| r.ok())
            .collect();
        Ok(tracks)
    }

    pub fn all_tracks(&self) -> Result<Vec<Track>, BoxError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, path, title, artist, album, track_number, duration_secs, file_hash, rarity, manually_edited
               FROM tracks
              ORDER BY artist, album, track_number, title",
        )?;
        let tracks = stmt
            .query_map([], row_to_track)?
            .filter_map(|r| r.ok())
            .collect();
        Ok(tracks)
    }

    pub fn reindex(&self) -> Result<(), BoxError> {
        let conn = self.conn.lock().unwrap();
        index_directory(&conn, &self.data_dir)
    }
}

// ── DB helpers ────────────────────────────────────────────────────────────────

fn init_schema(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS tracks (
            id            INTEGER PRIMARY KEY AUTOINCREMENT,
            path          TEXT    NOT NULL UNIQUE,
            title         TEXT,
            artist        TEXT,
            album         TEXT,
            track_number  INTEGER,
            duration_secs REAL,
            modified_secs INTEGER NOT NULL DEFAULT 0,
            cover_data    BLOB,
            cover_mime    TEXT
        );
        CREATE INDEX IF NOT EXISTS idx_artist ON tracks(artist COLLATE NOCASE);
        CREATE INDEX IF NOT EXISTS idx_album  ON tracks(album  COLLATE NOCASE);
        CREATE INDEX IF NOT EXISTS idx_title  ON tracks(title  COLLATE NOCASE);",
    )?;
    // Migrate existing databases that predate the cover columns.
    let _ = conn.execute_batch("ALTER TABLE tracks ADD COLUMN cover_data BLOB");
    let _ = conn.execute_batch("ALTER TABLE tracks ADD COLUMN cover_mime TEXT");
    let _ = conn.execute_batch("ALTER TABLE tracks ADD COLUMN file_hash TEXT");
    let _ = conn.execute_batch("ALTER TABLE tracks ADD COLUMN rarity TEXT");
    let _ = conn.execute_batch("ALTER TABLE tracks ADD COLUMN manually_edited INTEGER NOT NULL DEFAULT 0");
    Ok(())
}

fn row_to_track(row: &rusqlite::Row<'_>) -> rusqlite::Result<Track> {
    Ok(Track {
        id: row.get(0)?,
        path: row.get(1)?,
        title: row.get(2)?,
        artist: row.get(3)?,
        album: row.get(4)?,
        track_number: row.get(5)?,
        duration_secs: row.get(6)?,
        file_hash: row.get(7)?,
        rarity: row.get(8)?,
        manually_edited: row.get::<_, i64>(9).unwrap_or(0) != 0,
    })
}

// ── Indexing ──────────────────────────────────────────────────────────────────

fn index_directory(conn: &Connection, data_dir: &Path) -> Result<(), BoxError> {
    let mut visited: HashSet<String> = HashSet::new();

    for entry in WalkDir::new(data_dir)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
    {
        let path = entry.path();
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();

        if !AUDIO_EXTENSIONS.contains(&ext.as_str()) {
            continue;
        }

        let rel = rel_path(data_dir, path);
        visited.insert(rel);

        if let Err(e) = index_file(conn, data_dir, path) {
            eprintln!("[library] failed to index {}: {e}", path.display());
        }
    }

    // Remove DB rows whose files no longer exist.
    let stale: Vec<String> = {
        let mut stmt = conn.prepare("SELECT path FROM tracks")?;
        let stale = stmt
            .query_map([], |row| row.get(0))?
            .filter_map(|r| r.ok())
            .filter(|p: &String| !visited.contains(p))
            .collect();
        stale
    };
    for path in stale {
        conn.execute("DELETE FROM tracks WHERE path = ?1", params![path])?;
    }

    Ok(())
}

fn index_file(conn: &Connection, data_dir: &Path, abs: &Path) -> Result<(), BoxError> {
    let rel = rel_path(data_dir, abs);

    let modified_secs = abs
        .metadata()
        .and_then(|m| m.modified())
        .map(|t| {
            t.duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs() as i64
        })
        .unwrap_or(0);

    // Skip unchanged files that already have a hash.
    let cached: Option<(i64, Option<String>, bool)> = conn
        .query_row(
            "SELECT modified_secs, file_hash, manually_edited FROM tracks WHERE path = ?1",
            params![rel],
            |row| Ok((row.get(0)?, row.get(1)?, row.get::<_, i64>(2).unwrap_or(0) != 0)),
        )
        .ok();
    if modified_secs > 0 {
        if let Some((ms, Some(_), _)) = &cached {
            if *ms == modified_secs {
                return Ok(());
            }
        }
    }

    // If manually edited, only update file hash, rarity, duration, cover — preserve metadata.
    if let Some((_, _, true)) = &cached {
        let meta = read_audio_meta(abs);
        let file_hash = hash_file(abs);
        let rarity = file_hash.as_deref().map(rarity_from_hash);
        conn.execute(
            "UPDATE tracks SET modified_secs = ?1, file_hash = ?2, rarity = ?3,
             duration_secs = COALESCE(?4, duration_secs), cover_data = ?5, cover_mime = ?6
             WHERE path = ?7",
            params![modified_secs, file_hash, rarity, meta.duration_secs, meta.cover_data, meta.cover_mime, rel],
        )?;
        return Ok(());
    }

    let mut meta = read_audio_meta(abs);

    // Fall back to path / filename inference when the file has no tags.
    if meta.title.is_none() && meta.artist.is_none() {
        let duration = meta.duration_secs;
        let cover_data = meta.cover_data.take();
        let cover_mime = meta.cover_mime.take();
        meta = infer_from_path(&rel);
        meta.duration_secs = duration;
        meta.cover_data = cover_data;
        meta.cover_mime = cover_mime;
    }

    // Hash file contents for gacha rarity.
    let file_hash = hash_file(abs);
    let rarity = file_hash.as_deref().map(rarity_from_hash);

    conn.execute(
        "INSERT INTO tracks
             (path, title, artist, album, track_number, duration_secs, modified_secs, cover_data, cover_mime, file_hash, rarity)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
         ON CONFLICT(path) DO UPDATE SET
             title         = excluded.title,
             artist        = excluded.artist,
             album         = excluded.album,
             track_number  = excluded.track_number,
             duration_secs = excluded.duration_secs,
             modified_secs = excluded.modified_secs,
             cover_data    = excluded.cover_data,
             cover_mime    = excluded.cover_mime,
             file_hash     = excluded.file_hash,
             rarity        = excluded.rarity",
        params![
            rel,
            meta.title,
            meta.artist,
            meta.album,
            meta.track_number,
            meta.duration_secs,
            modified_secs,
            meta.cover_data,
            meta.cover_mime,
            file_hash,
            rarity,
        ],
    )?;

    Ok(())
}

/// Returns the path relative to `data_dir` with forward-slash separators.
fn rel_path(data_dir: &Path, abs: &Path) -> String {
    abs.strip_prefix(data_dir)
        .map(|p| p.to_string_lossy().replace('\\', "/"))
        .unwrap_or_else(|_| abs.to_string_lossy().to_string())
}

// ── Hashing & Gacha rarity ────────────────────────────────────────────────────

/// BLAKE3 hash of the full file contents, returned as a hex string.
fn hash_file(path: &Path) -> Option<String> {
    let data = std::fs::read(path).ok()?;
    Some(blake3::hash(&data).to_hex().to_string())
}

/// Deterministic rarity grade derived from the first byte of the hash.
///
/// The BLAKE3 hash is uniformly distributed, so the first byte gives a
/// fair 256-bucket lottery.  Tiers and their probabilities:
///
/// | Grade     | First byte | Probability |
/// |-----------|------------|-------------|
/// | Common    | 128–255    | 50.00 %     |
/// | Uncommon  |  64–127    | 25.00 %     |
/// | Rare      |  32–63     | 12.50 %     |
/// | Epic      |  12–31     |  7.81 %     |
/// | Legendary |   3–11     |  3.52 %     |
/// | Mythic    |   0–2      |  1.17 %     |
fn rarity_from_hash(hex: &str) -> String {
    let byte = u8::from_str_radix(&hex[..2], 16).unwrap_or(255);
    let grade = match byte {
        0..=2 => "Mythic",
        3..=11 => "Legendary",
        12..=31 => "Epic",
        32..=63 => "Rare",
        64..=127 => "Uncommon",
        128..=255 => "Common",
    };
    grade.to_owned()
}

// ── Metadata extraction ───────────────────────────────────────────────────────

/// Tries to read embedded tags + duration. Returns `Meta::default()` on failure.
fn read_audio_meta(path: &Path) -> Meta {
    let tagged = match Probe::open(path).ok().and_then(|p| p.read().ok()) {
        Some(t) => t,
        None => return Meta::default(),
    };

    let duration_secs = {
        let d = tagged.properties().duration().as_secs_f64();
        if d > 0.0 { Some(d) } else { None }
    };

    let tag = match tagged.primary_tag().or_else(|| tagged.first_tag()) {
        Some(t) => t,
        None => return Meta { duration_secs, ..Default::default() },
    };

    let (cover_data, cover_mime) = extract_cover(tag);

    Meta {
        title: tag.title().as_deref().map(str::to_owned),
        artist: tag.artist().as_deref().map(str::to_owned),
        album: tag.album().as_deref().map(str::to_owned),
        track_number: tag.track().map(|n| n as i64),
        duration_secs,
        cover_data,
        cover_mime,
    }
}

/// Extracts the first embedded cover image from a tag.
fn extract_cover(tag: &lofty::tag::Tag) -> (Option<Vec<u8>>, Option<String>) {
    let pictures = tag.pictures();
    let pic = pictures
        .iter()
        .find(|p| p.pic_type() == PictureType::CoverFront)
        .or_else(|| pictures.first());
    match pic {
        Some(p) => {
            let mime = p.mime_type().map(|m| m.to_string());
            (Some(p.data().to_vec()), mime)
        }
        None => (None, None),
    }
}

/// Infers artist / album / title from the relative path.
///
/// Patterns (path components before the filename):
///   - _(none)_          → title from filename
///   - `Artist/`         → artist + title
///   - `Artist/Album/`   → artist + album + title
///   - deeper nesting    → first two components as artist / album
fn infer_from_path(rel: &str) -> Meta {
    let path = Path::new(rel);

    let components: Vec<&str> = path
        .parent()
        .map(|p| {
            p.components()
                .filter_map(|c| c.as_os_str().to_str())
                .filter(|s| !s.is_empty())
                .collect()
        })
        .unwrap_or_default();

    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or(rel);

    let (artist, album) = match components.as_slice() {
        [] => (None, None),
        [a] => (Some((*a).to_owned()), None),
        [a, b, ..] => (Some((*a).to_owned()), Some((*b).to_owned())),
    };

    let (track_number, title) = parse_filename(stem);

    Meta {
        title: Some(title),
        artist,
        album,
        track_number,
        duration_secs: None,
        ..Default::default()
    }
}

/// Parses an optional track number and title from a filename stem.
///
/// Recognised patterns:
///   - `01 - Song Title`  → `(Some(1), "Song Title")`
///   - `01. Song Title`   → `(Some(1), "Song Title")`
///   - `01 Song Title`    → `(Some(1), "Song Title")`
///   - `Song Title`       → `(None,    "Song Title")`
fn parse_filename(stem: &str) -> (Option<i64>, String) {
    let bytes = stem.as_bytes();
    let mut i = 0;
    while i < bytes.len() && bytes[i].is_ascii_digit() {
        i += 1;
    }
    if i > 0 && i < stem.len() {
        let rest = stem[i..].trim_start_matches(|c: char| c == '.' || c == '-' || c == ' ');
        if !rest.is_empty() {
            if let Ok(n) = stem[..i].parse::<i64>() {
                return (Some(n), rest.to_owned());
            }
        }
    }
    (None, stem.to_owned())
}

// ── Filesystem watcher ────────────────────────────────────────────────────────

fn start_watcher(
    data_dir: PathBuf,
    conn: Arc<Mutex<Connection>>,
    app_handle: tauri::AppHandle,
) -> Result<(), BoxError> {
    let (tx, rx) = mpsc::channel::<notify::Result<Event>>();

    let mut watcher = RecommendedWatcher::new(
        move |res| {
            let _ = tx.send(res);
        },
        notify::Config::default(),
    )?;
    watcher.watch(&data_dir, RecursiveMode::Recursive)?;

    thread::spawn(move || {
        let _watcher = watcher; // keep watcher alive for the lifetime of this thread
        for result in rx {
            match result {
                Ok(event) => handle_fs_event(event, &conn, &data_dir, &app_handle),
                Err(e) => eprintln!("[library] watcher error: {e}"),
            }
        }
    });

    Ok(())
}

fn handle_fs_event(
    event: Event,
    conn: &Arc<Mutex<Connection>>,
    data_dir: &Path,
    app_handle: &tauri::AppHandle,
) {
    let mut changed = false;
    match event.kind {
        EventKind::Create(_) | EventKind::Modify(_) => {
            for path in &event.paths {
                let ext = path
                    .extension()
                    .and_then(|e| e.to_str())
                    .unwrap_or("")
                    .to_ascii_lowercase();
                if AUDIO_EXTENSIONS.contains(&ext.as_str()) {
                    let conn = conn.lock().unwrap();
                    println!("[library] watcher: indexing {}", path.display());
                    if let Err(e) = index_file(&conn, data_dir, path) {
                        eprintln!("[library] watcher: index error for {}: {e}", path.display());
                    } else {
                        changed = true;
                    }
                }
            }
        }
        EventKind::Remove(_) => {
            for path in &event.paths {
                let rel = rel_path(data_dir, path);
                let conn = conn.lock().unwrap();
                if conn.execute("DELETE FROM tracks WHERE path = ?1", params![rel]).unwrap_or(0) > 0 {
                    changed = true;
                }
            }
        }
        _ => {}
    }
    if changed {
        use tauri::Emitter;
        let _ = app_handle.emit("library-changed", ());
    }
}

// ── Tauri commands ────────────────────────────────────────────────────────────

/// Returns the cover image for a track as a base64-encoded data URL,
/// e.g. `data:image/jpeg;base64,...`, or `None` if no cover is stored.
#[tauri::command]
pub fn get_track_cover(
    id: i64,
    state: tauri::State<'_, LibraryState>,
) -> Result<Option<String>, String> {
    let conn = state.conn.lock().unwrap();
    let result: rusqlite::Result<(Option<Vec<u8>>, Option<String>)> = conn.query_row(
        "SELECT cover_data, cover_mime FROM tracks WHERE id = ?1",
        params![id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    );
    match result {
        Ok((Some(data), mime)) => {
            let mime = mime.unwrap_or_else(|| "image/jpeg".to_owned());
            Ok(Some(format!("data:{};base64,{}", mime, B64.encode(&data))))
        }
        Ok((None, _)) => Ok(None),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(e.to_string()),
    }
}

#[tauri::command]
pub fn search_tracks(
    query: String,
    state: tauri::State<'_, LibraryState>,
) -> Result<Vec<Track>, String> {
    state.search(&query).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_all_tracks(state: tauri::State<'_, LibraryState>) -> Result<Vec<Track>, String> {
    state.all_tracks().map_err(|e| e.to_string())
}

#[tauri::command]
pub fn reindex(state: tauri::State<'_, LibraryState>) -> Result<(), String> {
    state.reindex().map_err(|e| e.to_string())
}

#[tauri::command]
pub fn update_track(
    id: i64,
    title: Option<String>,
    artist: Option<String>,
    album: Option<String>,
    track_number: Option<i64>,
    state: tauri::State<'_, LibraryState>,
) -> Result<(), String> {
    let conn = state.conn.lock().unwrap();
    conn.execute(
        "UPDATE tracks SET title = ?1, artist = ?2, album = ?3, track_number = ?4, manually_edited = 1 WHERE id = ?5",
        params![title, artist, album, track_number, id],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub fn open_data_dir(state: tauri::State<'_, LibraryState>) -> Result<(), String> {
    open::that(&state.data_dir).map_err(|e| e.to_string())
}
