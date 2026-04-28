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
use std::time::Duration;
use walkdir::WalkDir;

type BoxError = Box<dyn std::error::Error + Send + Sync>;

const AUDIO_EXTENSIONS: &[&str] = &[
    "mp3", "flac", "ogg", "opus", "aac", "m4a", "wav", "wv", "ape",
];

const SIDECAR_COVER_FILENAMES: &[&str] = &[
    "cover.jpg",
    "cover.jpeg",
    "cover.png",
    "cover.webp",
    "folder.jpg",
    "folder.jpeg",
    "folder.png",
    "folder.webp",
    "front.jpg",
    "front.jpeg",
    "front.png",
    "front.webp",
    "album.jpg",
    "album.jpeg",
    "album.png",
    "album.webp",
    "art.jpg",
    "art.jpeg",
    "art.png",
    "art.webp",
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
    pub is_liked: bool,
    pub play_count: i64,
    pub year: Option<i64>,
    pub genre: Option<String>,
    pub date_added: Option<i64>,
}

#[derive(Debug, Serialize, Clone)]
pub struct DeviceSettings {
    pub emoji: String,
    pub device_name: String,
    pub soulseek_enabled: bool,
    pub soulseek_username: String,
    pub soulseek_password: String,
}

#[derive(Debug, Serialize, Clone)]
pub struct PlayHistoryEntry {
    pub played_at: i64,
    pub track: Track,
}

// ── Internal types ────────────────────────────────────────────────────────────

#[derive(Default)]
struct Meta {
    title: Option<String>,
    artist: Option<String>,
    album: Option<String>,
    track_number: Option<i64>,
    duration_secs: Option<f64>,
    year: Option<i64>,
    genre: Option<String>,
    /// Raw bytes of the embedded cover image.
    cover_data: Option<Vec<u8>>,
    cover_mime: Option<String>,
    cover_source_path: Option<String>,
    cover_source_mtime: i64,
}

struct SidecarCoverCandidate {
    abs_path: PathBuf,
    rel_path: String,
    modified_secs: i64,
    mime: String,
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

        let db_path = data_dir.join("app.db");
        let is_new_db = !db_path.exists();

        let conn = Connection::open(&db_path)?;
        init_schema(&conn)?;

        // Only run the full directory scan on first launch (new DB).
        // On subsequent launches the FS watcher handles incremental changes;
        // the user can trigger a manual reindex at any time.
        if is_new_db {
            index_directory(&conn, &data_dir)?;
        }

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
            "SELECT id, path, title, artist, album, track_number, duration_secs, file_hash, rarity, manually_edited, is_liked, play_count, year, genre, date_added
               FROM tracks
              WHERE title  LIKE ?1 COLLATE NOCASE
                 OR artist LIKE ?1 COLLATE NOCASE
                      OR album  LIKE ?1 COLLATE NOCASE
                      OR genre  LIKE ?1 COLLATE NOCASE
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
            "SELECT id, path, title, artist, album, track_number, duration_secs, file_hash, rarity, manually_edited, is_liked, play_count, year, genre, date_added
               FROM tracks
              ORDER BY artist, album, track_number, title",
        )?;
        let tracks = stmt
            .query_map([], row_to_track)?
            .filter_map(|r| r.ok())
            .collect();
        Ok(tracks)
    }

    pub fn reindex(&self, app: &tauri::AppHandle) {
        let conn = Arc::clone(&self.conn);
        let data_dir = self.data_dir.clone();
        let app = app.clone();
        thread::spawn(move || {
            index_directory_async(&conn, &data_dir, &app);
        });
    }

    pub fn get_device_settings(&self) -> Result<DeviceSettings, BoxError> {
        let conn = self.conn.lock().unwrap();
        let existing: Result<(String, String, i64, String, String), _> = conn.query_row(
            "SELECT emoji,
                    COALESCE(device_name, ''),
                    COALESCE(soulseek_enabled, 0),
                    COALESCE(soulseek_username, ''),
                    COALESCE(soulseek_password, '')
               FROM device_config
              WHERE id = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)),
        );

        match existing {
            Ok((emoji, device_name, soulseek_enabled, soulseek_username, soulseek_password)) => Ok(DeviceSettings {
                emoji,
                device_name,
                soulseek_enabled: soulseek_enabled != 0,
                soulseek_username,
                soulseek_password,
            }),
            Err(_) => {
                let emoji = random_emoji();
                let device_name = whoami::devicename().trim().to_string();
                conn.execute(
                    "INSERT OR REPLACE INTO device_config (
                        id,
                        emoji,
                        device_name,
                        soulseek_enabled,
                        soulseek_username,
                        soulseek_password
                    ) VALUES (1, ?1, ?2, 0, '', '')",
                    params![&emoji, &device_name],
                )?;
                Ok(DeviceSettings {
                    emoji,
                    device_name,
                    soulseek_enabled: false,
                    soulseek_username: String::new(),
                    soulseek_password: String::new(),
                })
            }
        }
    }

    pub fn set_device_settings(
        &self,
        emoji: &str,
        device_name: &str,
        soulseek_enabled: bool,
        soulseek_username: &str,
        soulseek_password: &str,
    ) -> Result<(), BoxError> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT OR REPLACE INTO device_config (
                id,
                emoji,
                device_name,
                soulseek_enabled,
                soulseek_username,
                soulseek_password
            ) VALUES (1, ?1, ?2, ?3, ?4, ?5)",
            params![
                emoji,
                device_name,
                if soulseek_enabled { 1 } else { 0 },
                soulseek_username.trim(),
                soulseek_password,
            ],
        )?;
        Ok(())
    }

    pub fn get_device_emoji(&self) -> Result<String, BoxError> {
        Ok(self.get_device_settings()?.emoji)
    }

    pub fn set_device_emoji(&self, emoji: &str) -> Result<(), BoxError> {
        let current = self.get_device_settings()?;
        self.set_device_settings(
            emoji,
            &current.device_name,
            current.soulseek_enabled,
            &current.soulseek_username,
            &current.soulseek_password,
        )
    }
}

fn random_emoji() -> String {
    const EMOJIS: &[&str] = &[
        "🎵", "🎶", "🎤", "🎧", "🎼", "🎹", "🎸", "🥁", "📱", "💻",
        "🖥️", "⌚", "📻", "📡", "🔊", "🎺", "🎻", "🪕", "🎷", "🍕",
    ];
    let idx = {
        use std::time::UNIX_EPOCH;
        let duration = UNIX_EPOCH.elapsed().unwrap_or_default();
        (duration.as_nanos() as usize) % EMOJIS.len()
    };
    EMOJIS[idx].to_string()
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
            cover_mime    TEXT,
            cover_source_path TEXT,
            cover_source_mtime INTEGER NOT NULL DEFAULT 0
        );
        CREATE INDEX IF NOT EXISTS idx_artist ON tracks(artist COLLATE NOCASE);
        CREATE INDEX IF NOT EXISTS idx_album  ON tracks(album  COLLATE NOCASE);
        CREATE INDEX IF NOT EXISTS idx_title  ON tracks(title  COLLATE NOCASE);",
    )?;
    // Migrate existing databases that predate the cover columns.
    let _ = conn.execute_batch("ALTER TABLE tracks ADD COLUMN cover_data BLOB");
    let _ = conn.execute_batch("ALTER TABLE tracks ADD COLUMN cover_mime TEXT");
    let _ = conn.execute_batch("ALTER TABLE tracks ADD COLUMN cover_source_path TEXT");
    let _ = conn.execute_batch("ALTER TABLE tracks ADD COLUMN cover_source_mtime INTEGER NOT NULL DEFAULT 0");
    let _ = conn.execute_batch("ALTER TABLE tracks ADD COLUMN file_hash TEXT");
    let _ = conn.execute_batch("ALTER TABLE tracks ADD COLUMN rarity TEXT");
    let _ = conn.execute_batch("ALTER TABLE tracks ADD COLUMN manually_edited INTEGER NOT NULL DEFAULT 0");
    let _ = conn.execute_batch("ALTER TABLE tracks ADD COLUMN is_liked INTEGER NOT NULL DEFAULT 0");
    let _ = conn.execute_batch("ALTER TABLE tracks ADD COLUMN play_count INTEGER NOT NULL DEFAULT 0");
    let _ = conn.execute_batch("ALTER TABLE tracks ADD COLUMN year INTEGER");
    let _ = conn.execute_batch("ALTER TABLE tracks ADD COLUMN genre TEXT");
    let _ = conn.execute_batch("ALTER TABLE tracks ADD COLUMN date_added INTEGER");
    conn.execute_batch(
        "CREATE INDEX IF NOT EXISTS idx_genre ON tracks(genre COLLATE NOCASE);",
    )?;
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS play_history (
            id        INTEGER PRIMARY KEY AUTOINCREMENT,
            track_id  INTEGER NOT NULL REFERENCES tracks(id) ON DELETE CASCADE,
            played_at INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_played_at ON play_history(played_at DESC);
        CREATE TABLE IF NOT EXISTS device_config (
            id        INTEGER PRIMARY KEY CHECK(id=1),
            emoji     TEXT NOT NULL DEFAULT '🎵',
            device_name TEXT NOT NULL DEFAULT '',
            soulseek_enabled INTEGER NOT NULL DEFAULT 0,
            soulseek_username TEXT NOT NULL DEFAULT '',
            soulseek_password TEXT NOT NULL DEFAULT ''
        );
        CREATE TABLE IF NOT EXISTS playlists (
            id         INTEGER PRIMARY KEY AUTOINCREMENT,
            name       TEXT NOT NULL,
            created_at INTEGER NOT NULL DEFAULT (strftime('%s','now'))
        );
        CREATE TABLE IF NOT EXISTS playlist_tracks (
            id          INTEGER PRIMARY KEY AUTOINCREMENT,
            playlist_id INTEGER NOT NULL REFERENCES playlists(id) ON DELETE CASCADE,
            track_id    INTEGER NOT NULL REFERENCES tracks(id) ON DELETE CASCADE,
            position    INTEGER NOT NULL DEFAULT 0,
            UNIQUE(playlist_id, track_id)
        );
        CREATE INDEX IF NOT EXISTS idx_pt_playlist ON playlist_tracks(playlist_id, position);",
    )?;
    let _ = conn.execute_batch("ALTER TABLE device_config ADD COLUMN device_name TEXT NOT NULL DEFAULT ''");
    let _ = conn.execute_batch("ALTER TABLE device_config ADD COLUMN soulseek_enabled INTEGER NOT NULL DEFAULT 0");
    let _ = conn.execute_batch("ALTER TABLE device_config ADD COLUMN soulseek_username TEXT NOT NULL DEFAULT ''");
    let _ = conn.execute_batch("ALTER TABLE device_config ADD COLUMN soulseek_password TEXT NOT NULL DEFAULT ''");
    // Smart (flexible) playlists — previously in localStorage, now in DB for sync.
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS smart_playlists (
            id         TEXT PRIMARY KEY,
            name       TEXT NOT NULL,
            match_mode TEXT NOT NULL DEFAULT 'all',
            rules_json TEXT NOT NULL DEFAULT '[]',
            updated_at INTEGER NOT NULL DEFAULT (strftime('%s','now'))
        );",
    )?;
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
        is_liked: row.get::<_, i64>(10).unwrap_or(0) != 0,
        play_count: row.get::<_, i64>(11).unwrap_or(0),
        year: row.get(12).unwrap_or(None),
        genre: row.get(13).unwrap_or(None),
        date_added: row.get(14).unwrap_or(None),
    })
}

// ── Indexing ──────────────────────────────────────────────────────────────────

#[derive(Clone, Serialize)]
struct IndexProgress {
    current: usize,
    total: usize,
    status: String, // indexing, done
    added: usize,
    track_name: Option<String>,
}

fn emit_index_progress(
    app: &tauri::AppHandle,
    current: usize,
    total: usize,
    status: &str,
    added: usize,
    track_name: Option<String>,
) {
    use tauri::Emitter;
    let _ = app.emit("index-progress", IndexProgress {
        current,
        total,
        status: status.to_owned(),
        added,
        track_name,
    });
}

/// Async version that emits progress events.
fn index_directory_async(
    conn: &Arc<Mutex<Connection>>,
    data_dir: &Path,
    app: &tauri::AppHandle,
) {
    // Collect all audio files first.
    let files: Vec<PathBuf> = WalkDir::new(data_dir)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
        .filter(|e| {
            let ext = e.path().extension().and_then(|x| x.to_str()).unwrap_or("").to_ascii_lowercase();
            AUDIO_EXTENSIONS.contains(&ext.as_str())
        })
        .map(|e| e.into_path())
        .collect();

    let total = files.len();
    let mut visited: HashSet<String> = HashSet::new();
    let mut added: usize = 0;

    for (i, path) in files.iter().enumerate() {
        let rel = rel_path(data_dir, path);
        visited.insert(rel.clone());

        let conn = conn.lock().unwrap();
        match index_file(&conn, data_dir, path) {
            Ok(true) => {
                added += 1;
                emit_index_progress(
                    app,
                    i + 1,
                    total,
                    "added",
                    added,
                    Some(track_name_from_rel(&rel)),
                );
            }
            Err(e) => eprintln!("[library] failed to index {}: {e}", path.display()),
            _ => {}
        }
        drop(conn);

        // Emit progress every 5 files or at the end.
        if (i + 1) % 5 == 0 || i + 1 == total {
            emit_index_progress(app, i + 1, total, "indexing", added, None);
        }
    }

    // Remove stale rows.
    {
        let conn = conn.lock().unwrap();
        let stale: Vec<String> = {
            let mut stmt = conn.prepare("SELECT path FROM tracks").unwrap_or_else(|_| unreachable!());
            stmt.query_map([], |row| row.get(0))
                .unwrap_or_else(|_| unreachable!())
                .filter_map(|r| r.ok())
                .filter(|p: &String| !visited.contains(p))
                .collect()
        };
        for path in &stale {
            let _ = conn.execute("DELETE FROM tracks WHERE path = ?1", params![path]);
        }
        if !stale.is_empty() {
            added += stale.len(); // count removals as changes too
        }
    }

    emit_index_progress(app, total, total, "done", added, None);

    use tauri::Emitter;
    let _ = app.emit("library-changed", ());
}

/// Synchronous version for startup (no events available yet).
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
    // Guard: if the walk found nothing at all (permission denied, unmounted storage, etc.)
    // do NOT purge the DB — that would wipe all tracks on every cold start.
    if visited.is_empty() {
        return Ok(());
    }
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

/// Returns `Ok(true)` if a new/updated row was written, `Ok(false)` if skipped.
fn index_file(conn: &Connection, data_dir: &Path, abs: &Path) -> Result<bool, BoxError> {
    let rel = rel_path(data_dir, abs);
    let sidecar_cover = find_sidecar_cover_candidate(data_dir, abs);

    let modified_secs = path_modified_secs(abs);

    // Skip unchanged files that already have a hash.
    let cached: Option<(i64, Option<String>, bool, Option<String>, i64)> = conn
        .query_row(
            "SELECT modified_secs,
                    file_hash,
                    manually_edited,
                    cover_source_path,
                    COALESCE(cover_source_mtime, 0)
               FROM tracks
              WHERE path = ?1",
            params![rel],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get::<_, i64>(2).unwrap_or(0) != 0,
                    row.get(3)?,
                    row.get(4)?,
                ))
            },
        )
        .ok();
    if modified_secs > 0 {
        if let Some((ms, Some(_), _, cached_cover_source_path, cached_cover_source_mtime)) = &cached {
            if *ms == modified_secs
                && sidecar_cover_state_matches(
                    cached_cover_source_path.as_deref(),
                    *cached_cover_source_mtime,
                    sidecar_cover.as_ref(),
                )
            {
                return Ok(false);
            }
        }
    }

    // If manually edited, only update file hash, rarity, duration, cover — preserve metadata.
    if let Some((_, _, true, _, _)) = &cached {
        let meta = read_audio_meta(abs, sidecar_cover.as_ref());
        let file_hash = hash_file(abs);
        let rarity = file_hash.as_deref().map(rarity_from_hash);
        conn.execute(
            "UPDATE tracks SET modified_secs = ?1, file_hash = ?2, rarity = ?3,
             duration_secs = COALESCE(?4, duration_secs), cover_data = ?5, cover_mime = ?6,
             cover_source_path = ?7, cover_source_mtime = ?8
             WHERE path = ?9",
            params![
                modified_secs,
                file_hash,
                rarity,
                meta.duration_secs,
                meta.cover_data,
                meta.cover_mime,
                meta.cover_source_path,
                meta.cover_source_mtime,
                rel,
            ],
        )?;
        return Ok(true);
    }

    let mut meta = read_audio_meta(abs, sidecar_cover.as_ref());

    // Fall back to path / filename inference when the file has no tags.
    if meta.title.is_none() && meta.artist.is_none() {
        let duration = meta.duration_secs;
        let cover_data = meta.cover_data.take();
        let cover_mime = meta.cover_mime.take();
        let cover_source_path = meta.cover_source_path.take();
        let cover_source_mtime = meta.cover_source_mtime;
        meta = infer_from_path(&rel);
        meta.duration_secs = duration;
        meta.cover_data = cover_data;
        meta.cover_mime = cover_mime;
        meta.cover_source_path = cover_source_path;
        meta.cover_source_mtime = cover_source_mtime;
    }

    // Hash file contents for gacha rarity.
    let file_hash = hash_file(abs);
    let rarity = file_hash.as_deref().map(rarity_from_hash);

    let now_secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;

    conn.execute(
        "INSERT INTO tracks
             (path, title, artist, album, track_number, duration_secs, modified_secs, cover_data, cover_mime, cover_source_path, cover_source_mtime, file_hash, rarity, year, genre, date_added)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)
         ON CONFLICT(path) DO UPDATE SET
             title         = excluded.title,
             artist        = excluded.artist,
             album         = excluded.album,
             track_number  = excluded.track_number,
             duration_secs = excluded.duration_secs,
             modified_secs = excluded.modified_secs,
             cover_data    = excluded.cover_data,
             cover_mime    = excluded.cover_mime,
             cover_source_path = excluded.cover_source_path,
             cover_source_mtime = excluded.cover_source_mtime,
             file_hash     = excluded.file_hash,
             rarity        = excluded.rarity,
             year          = excluded.year,
             genre         = excluded.genre",
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
            meta.cover_source_path,
            meta.cover_source_mtime,
            file_hash,
            rarity,
            meta.year,
            meta.genre,
            now_secs,
        ],
    )?;

    Ok(true)
}

/// Returns the path relative to `data_dir` with forward-slash separators.
fn rel_path(data_dir: &Path, abs: &Path) -> String {
    abs.strip_prefix(data_dir)
        .map(|p| p.to_string_lossy().replace('\\', "/"))
        .unwrap_or_else(|_| abs.to_string_lossy().to_string())
}

fn track_name_from_rel(rel: &str) -> String {
    Path::new(rel)
        .file_stem()
        .and_then(|s| s.to_str())
        .filter(|s| !s.is_empty())
        .unwrap_or(rel)
        .to_string()
}

fn path_modified_secs(path: &Path) -> i64 {
    path.metadata()
        .and_then(|metadata| metadata.modified())
        .map(|timestamp| {
            timestamp
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs() as i64
        })
        .unwrap_or(0)
}

fn sidecar_cover_state_matches(
    cached_cover_source_path: Option<&str>,
    cached_cover_source_mtime: i64,
    current_sidecar_cover: Option<&SidecarCoverCandidate>,
) -> bool {
    match (cached_cover_source_path, current_sidecar_cover) {
        (None, None) => true,
        (Some(cached_path), Some(current)) => {
            cached_path == current.rel_path && cached_cover_source_mtime == current.modified_secs
        }
        _ => false,
    }
}

fn find_sidecar_cover_candidate(data_dir: &Path, audio_path: &Path) -> Option<SidecarCoverCandidate> {
    let directory = audio_path.parent()?;
    let mut best_match: Option<(usize, PathBuf)> = None;

    for entry in std::fs::read_dir(directory).ok()? {
        let entry = entry.ok()?;
        if !entry.file_type().ok()?.is_file() {
            continue;
        }

        let file_name = entry.file_name();
        let file_name = file_name.to_str()?.to_ascii_lowercase();
        let Some(priority) = SIDECAR_COVER_FILENAMES
            .iter()
            .position(|candidate| *candidate == file_name)
        else {
            continue;
        };

        match &best_match {
            Some((current_priority, _)) if *current_priority <= priority => {}
            _ => best_match = Some((priority, entry.path())),
        }
    }

    let (_, abs_path) = best_match?;
    let mime = sidecar_cover_mime(&abs_path)?;
    Some(SidecarCoverCandidate {
        rel_path: rel_path(data_dir, &abs_path),
        modified_secs: path_modified_secs(&abs_path),
        abs_path,
        mime,
    })
}

fn read_sidecar_cover(
    sidecar_cover: Option<&SidecarCoverCandidate>,
) -> (Option<Vec<u8>>, Option<String>, Option<String>, i64) {
    let Some(sidecar_cover) = sidecar_cover else {
        return (None, None, None, 0);
    };

    match std::fs::read(&sidecar_cover.abs_path) {
        Ok(data) => (
            Some(data),
            Some(sidecar_cover.mime.clone()),
            Some(sidecar_cover.rel_path.clone()),
            sidecar_cover.modified_secs,
        ),
        Err(_) => (None, None, None, 0),
    }
}

fn sidecar_cover_mime(path: &Path) -> Option<String> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    let mime = match ext.as_str() {
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        "webp" => "image/webp",
        "gif" => "image/gif",
        _ => return None,
    };
    Some(mime.to_string())
}

fn is_sidecar_cover_path(path: &Path) -> bool {
    path.file_name()
        .and_then(|file_name| file_name.to_str())
        .map(|file_name| file_name.to_ascii_lowercase())
        .map(|file_name| SIDECAR_COVER_FILENAMES.contains(&file_name.as_str()))
        .unwrap_or(false)
}

fn sibling_audio_files(directory: &Path) -> Vec<PathBuf> {
    let mut audio_files = Vec::new();

    let entries = match std::fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(_) => return audio_files,
    };

    for entry in entries.filter_map(Result::ok) {
        if !entry.file_type().map(|file_type| file_type.is_file()).unwrap_or(false) {
            continue;
        }

        let path = entry.path();
        let ext = path
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        if AUDIO_EXTENSIONS.contains(&ext.as_str()) {
            audio_files.push(path);
        }
    }

    audio_files
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

/// Tries to read embedded tags + duration. Falls back to adjacent cover files when present.
fn read_audio_meta(path: &Path, sidecar_cover: Option<&SidecarCoverCandidate>) -> Meta {
    let tagged = match Probe::open(path).ok().and_then(|p| p.read().ok()) {
        Some(t) => t,
        None => {
            let (cover_data, cover_mime, cover_source_path, cover_source_mtime) =
                read_sidecar_cover(sidecar_cover);
            return Meta {
                cover_data,
                cover_mime,
                cover_source_path,
                cover_source_mtime,
                ..Default::default()
            };
        }
    };

    let duration_secs = {
        let d = tagged.properties().duration().as_secs_f64();
        if d > 0.0 { Some(d) } else { None }
    };

    let tag = match tagged.primary_tag().or_else(|| tagged.first_tag()) {
        Some(t) => t,
        None => {
            let (cover_data, cover_mime, cover_source_path, cover_source_mtime) =
                read_sidecar_cover(sidecar_cover);
            return Meta {
                duration_secs,
                cover_data,
                cover_mime,
                cover_source_path,
                cover_source_mtime,
                ..Default::default()
            };
        }
    };

    let (cover_data, cover_mime, cover_source_path, cover_source_mtime) =
        extract_cover(tag, sidecar_cover);

    Meta {
        title: tag.title().as_deref().map(str::to_owned),
        artist: tag.artist().as_deref().map(str::to_owned),
        album: tag.album().as_deref().map(str::to_owned),
        track_number: tag.track().map(|n| n as i64),
        duration_secs,
        year: tag.date().map(|ts| ts.year as i64),
        genre: tag.genre().as_deref().map(str::to_owned),
        cover_data,
        cover_mime,
        cover_source_path,
        cover_source_mtime,
    }
}

/// Extracts the first embedded cover image from a tag, falling back to sidecar folder art.
fn extract_cover(
    tag: &lofty::tag::Tag,
    sidecar_cover: Option<&SidecarCoverCandidate>,
) -> (Option<Vec<u8>>, Option<String>, Option<String>, i64) {
    let pictures = tag.pictures();
    let pic = pictures
        .iter()
        .find(|p| p.pic_type() == PictureType::CoverFront)
        .or_else(|| pictures.first());
    match pic {
        Some(p) => {
            let mime = p.mime_type().map(|m| m.to_string());
            (Some(p.data().to_vec()), mime, None, 0)
        }
        None => read_sidecar_cover(sidecar_cover),
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
        let _watcher = watcher;
        // Collect events and debounce: wait 300ms of silence before processing batch.
        loop {
            // Block until first event.
            let first = match rx.recv() {
                Ok(r) => r,
                Err(_) => break,
            };
            let mut events = vec![first];
            // Drain any further events within the debounce window.
            loop {
                match rx.recv_timeout(Duration::from_millis(300)) {
                    Ok(e) => events.push(e),
                    Err(mpsc::RecvTimeoutError::Timeout) => break,
                    Err(mpsc::RecvTimeoutError::Disconnected) => break,
                }
            }
            handle_fs_events_batch(events, &conn, &data_dir, &app_handle);
        }
    });

    Ok(())
}

fn handle_fs_events_batch(
    events: Vec<notify::Result<Event>>,
    conn: &Arc<Mutex<Connection>>,
    data_dir: &Path,
    app_handle: &tauri::AppHandle,
) {
    let mut to_index: Vec<PathBuf> = Vec::new();
    let mut to_remove: Vec<PathBuf> = Vec::new();

    for result in events {
        let event = match result {
            Ok(e) => e,
            Err(e) => { eprintln!("[library] watcher error: {e}"); continue; }
        };
        match event.kind {
            EventKind::Create(_) | EventKind::Modify(_) => {
                for path in event.paths {
                    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
                    if AUDIO_EXTENSIONS.contains(&ext.as_str()) {
                        to_index.push(path);
                    } else if is_sidecar_cover_path(&path) {
                        if let Some(directory) = path.parent() {
                            to_index.extend(sibling_audio_files(directory));
                        }
                    }
                }
            }
            EventKind::Remove(_) => {
                for path in event.paths {
                    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
                    if AUDIO_EXTENSIONS.contains(&ext.as_str()) {
                        to_remove.push(path);
                    } else if is_sidecar_cover_path(&path) {
                        if let Some(directory) = path.parent() {
                            to_index.extend(sibling_audio_files(directory));
                        }
                    }
                }
            }
            _ => {}
        }
    }

    // Deduplicate
    to_index.sort();
    to_index.dedup();

    let total = to_index.len() + to_remove.len();
    if total == 0 { return; }

    let mut added: usize = 0;
    let mut current: usize = 0;

    for path in &to_index {
        let rel = rel_path(data_dir, path);
        current += 1;
        let c = conn.lock().unwrap();
        match index_file(&c, data_dir, path) {
            Ok(true) => {
                added += 1;
                emit_index_progress(
                    app_handle,
                    current,
                    total,
                    "added",
                    added,
                    Some(track_name_from_rel(&rel)),
                );
            }
            Err(e) => eprintln!("[library] watcher: index error for {}: {e}", path.display()),
            _ => {}
        }
        drop(c);
        if current % 5 == 0 || current == total {
            emit_index_progress(app_handle, current, total, "indexing", added, None);
        }
    }

    for path in &to_remove {
        let rel = rel_path(data_dir, path);
        let c = conn.lock().unwrap();
        if c.execute("DELETE FROM tracks WHERE path = ?1", params![rel]).unwrap_or(0) > 0 {
            added += 1;
        }
        drop(c);
        current += 1;
    }

    emit_index_progress(app_handle, total, total, "done", added, None);

    use tauri::Emitter;
    let _ = app_handle.emit("library-changed", ());
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
pub fn reindex(
    app: tauri::AppHandle,
    state: tauri::State<'_, LibraryState>,
) -> Result<(), String> {
    state.reindex(&app);
    Ok(())
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
    let _ = conn.execute(
        "UPDATE tracks SET play_count = play_count + 1 WHERE id = ?1",
        rusqlite::params![id],
    ).map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub fn get_data_dir(state: tauri::State<'_, LibraryState>) -> String {
    state.data_dir.to_string_lossy().into_owned()
}

#[tauri::command]
pub fn record_play(
    id: i64,
    state: tauri::State<'_, LibraryState>,
) -> Result<(), String> {
    let conn = state.conn.lock().unwrap();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;
    conn.execute(
        "INSERT INTO play_history (track_id, played_at) VALUES (?1, ?2)",
        params![id, now],
    )
    .map_err(|e| e.to_string())?;
    let _ = conn.execute(
        "UPDATE tracks SET play_count = play_count + 1 WHERE id = ?1",
        rusqlite::params![id],
    ).map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub fn get_recent_tracks(
    limit: Option<usize>,
    state: tauri::State<'_, LibraryState>,
) -> Result<Vec<Track>, String> {
    let conn = state.conn.lock().unwrap();
    let lim = limit.unwrap_or(12) as i64;
    let mut stmt = conn.prepare(
        "SELECT DISTINCT t.id, t.path, t.title, t.artist, t.album, t.track_number,
                t.duration_secs, t.file_hash, t.rarity, t.manually_edited, t.is_liked, t.play_count, t.year, t.genre
           FROM play_history h
           JOIN tracks t ON t.id = h.track_id
          ORDER BY h.played_at DESC
          LIMIT ?1",
    ).map_err(|e| e.to_string())?;
    let tracks = stmt
        .query_map(params![lim], row_to_track)
        .map_err(|e| e.to_string())?
        .filter_map(|r| r.ok())
        .collect();
    Ok(tracks)
}

#[tauri::command]
pub fn get_play_history(
    limit: Option<usize>,
    state: tauri::State<'_, LibraryState>,
) -> Result<Vec<PlayHistoryEntry>, String> {
    let conn = state.conn.lock().unwrap();
    let lim = limit.unwrap_or(500) as i64;
    let mut stmt = conn.prepare(
        "SELECT h.played_at, t.id, t.path, t.title, t.artist, t.album, t.track_number,
                t.duration_secs, t.file_hash, t.rarity, t.manually_edited, t.is_liked, t.play_count, t.year, t.genre, t.date_added
           FROM play_history h
           JOIN tracks t ON t.id = h.track_id
          ORDER BY h.played_at DESC
          LIMIT ?1",
    ).map_err(|e| e.to_string())?;
    let entries = stmt
        .query_map(params![lim], |row| {
            let played_at: i64 = row.get(0)?;
            let track = Track {
                id: row.get(1)?,
                path: row.get(2)?,
                title: row.get(3)?,
                artist: row.get(4)?,
                album: row.get(5)?,
                track_number: row.get(6)?,
                duration_secs: row.get(7)?,
                file_hash: row.get(8)?,
                rarity: row.get(9)?,
                manually_edited: row.get::<_, i64>(10).unwrap_or(0) != 0,
                is_liked: row.get::<_, i64>(11).unwrap_or(0) != 0,
                play_count: row.get::<_, i64>(12).unwrap_or(0),
                year: row.get(13).unwrap_or(None),
                genre: row.get(14).unwrap_or(None),
                date_added: row.get(15).unwrap_or(None),
            };
            Ok(PlayHistoryEntry { played_at, track })
        })
        .map_err(|e| e.to_string())?
        .filter_map(|r| r.ok())
        .collect();
    Ok(entries)
}

#[tauri::command]
pub fn get_device_emoji(state: tauri::State<'_, LibraryState>) -> Result<String, String> {
    state.get_device_emoji().map_err(|e| e.to_string())
}

#[tauri::command]
pub fn set_device_emoji(
    emoji: String,
    state: tauri::State<'_, LibraryState>,
) -> Result<(), String> {
    state.set_device_emoji(&emoji).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_device_settings(state: tauri::State<'_, LibraryState>) -> Result<DeviceSettings, String> {
    state.get_device_settings().map_err(|e| e.to_string())
}

#[tauri::command]
pub fn set_device_settings(
    emoji: String,
    device_name: String,
    soulseek_enabled: Option<bool>,
    soulseek_username: Option<String>,
    soulseek_password: Option<String>,
    state: tauri::State<'_, LibraryState>,
) -> Result<(), String> {
    let current = state.get_device_settings().map_err(|e| e.to_string())?;
    state
        .set_device_settings(
            &emoji,
            device_name.trim(),
            soulseek_enabled.unwrap_or(current.soulseek_enabled),
            soulseek_username
                .as_deref()
                .unwrap_or(&current.soulseek_username),
            soulseek_password
                .as_deref()
                .unwrap_or(&current.soulseek_password),
        )
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn toggle_like(id: i64, state: tauri::State<'_, LibraryState>) -> Result<bool, String> {
    let conn = state.conn.lock().unwrap();
    let current_liked: i64 = conn.query_row(
        "SELECT is_liked FROM tracks WHERE id = ?1",
        rusqlite::params![id],
        |row| row.get(0),
    ).unwrap_or(0);
    
    let new_liked = if current_liked == 0 { 1 } else { 0 };
    
    conn.execute(
        "UPDATE tracks SET is_liked = ?1 WHERE id = ?2",
        rusqlite::params![new_liked, id],
    ).map_err(|e| e.to_string())?;
    
    Ok(new_liked != 0)
}

// ── Playlists ─────────────────────────────────────────────────────────────────

#[derive(Debug, Serialize, Clone)]
pub struct Playlist {
    pub id: i64,
    pub name: String,
    pub created_at: i64,
    pub track_count: i64,
}

#[tauri::command]
pub fn get_playlists(state: tauri::State<'_, LibraryState>) -> Result<Vec<Playlist>, String> {
    let conn = state.conn.lock().unwrap();
    let mut stmt = conn.prepare(
        "SELECT p.id, p.name, p.created_at, COUNT(pt.id) as track_count
         FROM playlists p
         LEFT JOIN playlist_tracks pt ON pt.playlist_id = p.id
         GROUP BY p.id ORDER BY p.created_at DESC",
    ).map_err(|e| e.to_string())?;
    let rows = stmt.query_map([], |row| Ok(Playlist {
        id: row.get(0)?,
        name: row.get(1)?,
        created_at: row.get(2)?,
        track_count: row.get(3)?,
    })).map_err(|e| e.to_string())?;
    rows.map(|r| r.map_err(|e| e.to_string())).collect()
}

#[tauri::command]
pub fn create_playlist(name: String, state: tauri::State<'_, LibraryState>) -> Result<i64, String> {
    let conn = state.conn.lock().unwrap();
    conn.execute(
        "INSERT INTO playlists (name) VALUES (?1)",
        params![name],
    ).map_err(|e| e.to_string())?;
    Ok(conn.last_insert_rowid())
}

#[tauri::command]
pub fn rename_playlist(id: i64, name: String, state: tauri::State<'_, LibraryState>) -> Result<(), String> {
    let conn = state.conn.lock().unwrap();
    conn.execute(
        "UPDATE playlists SET name = ?1 WHERE id = ?2",
        params![name, id],
    ).map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub fn delete_playlist(id: i64, state: tauri::State<'_, LibraryState>) -> Result<(), String> {
    let conn = state.conn.lock().unwrap();
    conn.execute("DELETE FROM playlists WHERE id = ?1", params![id])
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub fn get_playlist_tracks(playlist_id: i64, state: tauri::State<'_, LibraryState>) -> Result<Vec<Track>, String> {
    let conn = state.conn.lock().unwrap();
    let mut stmt = conn.prepare(
        "SELECT t.id, t.path, t.title, t.artist, t.album, t.track_number,
                t.duration_secs, t.file_hash, t.rarity, t.manually_edited,
                t.is_liked, t.play_count, t.year, t.genre
         FROM tracks t
         JOIN playlist_tracks pt ON pt.track_id = t.id
         WHERE pt.playlist_id = ?1
         ORDER BY pt.position, pt.id",
    ).map_err(|e| e.to_string())?;
    let rows = stmt.query_map(params![playlist_id], row_to_track)
        .map_err(|e| e.to_string())?;
    rows.map(|r| r.map_err(|e| e.to_string())).collect()
}

#[tauri::command]
pub fn add_track_to_playlist(playlist_id: i64, track_id: i64, state: tauri::State<'_, LibraryState>) -> Result<(), String> {
    let conn = state.conn.lock().unwrap();
    let max_pos: i64 = conn.query_row(
        "SELECT COALESCE(MAX(position), -1) FROM playlist_tracks WHERE playlist_id = ?1",
        params![playlist_id],
        |row| row.get(0),
    ).unwrap_or(-1);
    conn.execute(
        "INSERT OR IGNORE INTO playlist_tracks (playlist_id, track_id, position) VALUES (?1, ?2, ?3)",
        params![playlist_id, track_id, max_pos + 1],
    ).map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub fn remove_track_from_playlist(playlist_id: i64, track_id: i64, state: tauri::State<'_, LibraryState>) -> Result<(), String> {
    let conn = state.conn.lock().unwrap();
    conn.execute(
        "DELETE FROM playlist_tracks WHERE playlist_id = ?1 AND track_id = ?2",
        params![playlist_id, track_id],
    ).map_err(|e| e.to_string())?;
    Ok(())
}

// ── Smart (flexible) Playlists ───────────────────────────────────────────────

#[derive(Debug, Serialize, serde::Deserialize, Clone)]
pub struct SmartPlaylistRow {
    pub id: String,
    pub name: String,
    pub match_mode: String, // "all" | "any"
    pub rules_json: String, // JSON array of rule objects
    pub updated_at: i64,
}

#[tauri::command]
pub fn get_smart_playlists(state: tauri::State<'_, LibraryState>) -> Result<Vec<SmartPlaylistRow>, String> {
    let conn = state.conn.lock().unwrap();
    let mut stmt = conn.prepare(
        "SELECT id, name, match_mode, rules_json, updated_at FROM smart_playlists ORDER BY name",
    ).map_err(|e| e.to_string())?;
    let rows = stmt.query_map([], |row| Ok(SmartPlaylistRow {
        id: row.get(0)?,
        name: row.get(1)?,
        match_mode: row.get(2)?,
        rules_json: row.get(3)?,
        updated_at: row.get(4)?,
    })).map_err(|e| e.to_string())?;
    rows.map(|r| r.map_err(|e| e.to_string())).collect()
}

#[tauri::command]
pub fn save_smart_playlist(
    id: String,
    name: String,
    match_mode: String,
    rules_json: String,
    state: tauri::State<'_, LibraryState>,
) -> Result<(), String> {
    let conn = state.conn.lock().unwrap();
    conn.execute(
        "INSERT INTO smart_playlists (id, name, match_mode, rules_json, updated_at)
         VALUES (?1, ?2, ?3, ?4, strftime('%s','now'))
         ON CONFLICT(id) DO UPDATE SET
           name = excluded.name,
           match_mode = excluded.match_mode,
           rules_json = excluded.rules_json,
           updated_at = strftime('%s','now')",
        params![id, name, match_mode, rules_json],
    ).map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub fn delete_smart_playlist(id: String, state: tauri::State<'_, LibraryState>) -> Result<(), String> {
    let conn = state.conn.lock().unwrap();
    conn.execute("DELETE FROM smart_playlists WHERE id = ?1", params![id])
        .map_err(|e| e.to_string())?;
    Ok(())
}
