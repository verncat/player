//! Peer sync subsystem.
//!
//! When sync is enabled:
//!   • Binds a small HTTP server on SYNC_PORT (57322).
//!       GET /tracks        → JSON { version, device_name, tracks:[...] }
//!       GET /file/<hash>   → raw audio bytes
//!   • `sync_with_peer` downloads every hash the peer has that we don't,
//!     skipping hash-collisions (deduplication by blake3 hash).
//!
//! Tauri event emitted to frontend: `"sync-progress"`
//!   { peer, phase: "index"|"download"|"reindex"|"done"|"error", total, done, message }

use std::collections::HashSet;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{IpAddr, SocketAddr, TcpListener, TcpStream, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, State};

pub const SYNC_PORT: u16 = 57322;
pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

// ── Public types ──────────────────────────────────────────────────────────────

/// Track info exchanged with peers.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct RemoteTrack {
    pub hash: String,
    pub path: String,
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
}

/// `/tracks` response payload for sync protocol.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct TracksResponse {
    pub version: String,
    pub device_name: Option<String>,
    pub tracks: Vec<RemoteTrack>,
}

/// Progress event payload.
#[derive(Serialize, Clone, Debug)]
pub struct SyncProgress {
    pub peer: String,
    pub device_name: Option<String>,
    /// "index" | "download" | "reindex" | "done" | "error"
    pub phase: String,
    pub total: usize,
    pub done: usize,
    pub message: Option<String>,
}

// ── State ─────────────────────────────────────────────────────────────────────

pub struct SyncState {
    inner: Arc<Mutex<SyncInner>>,
}

struct SyncInner {
    enabled: bool,
    server_started: bool,
}

impl SyncState {
    pub fn new() -> Self {
        SyncState {
            inner: Arc::new(Mutex::new(SyncInner {
                enabled: false,
                server_started: false,
            })),
        }
    }
}

// ── HTTP server (serves our tracks to peers) ──────────────────────────────────

fn start_http_server(conn: Arc<Mutex<Connection>>, data_dir: PathBuf) {
    thread::spawn(move || {
        let listener = match TcpListener::bind(("0.0.0.0", SYNC_PORT)) {
            Ok(l) => l,
            Err(e) => {
                eprintln!("[sync] cannot bind port {SYNC_PORT}: {e}");
                return;
            }
        };
        eprintln!("[sync] HTTP server ready on :{SYNC_PORT}");
        for stream in listener.incoming().flatten() {
            let conn = Arc::clone(&conn);
            let dir = data_dir.clone();
            thread::spawn(move || handle_http(stream, conn, dir));
        }
    });
}

fn handle_http(stream: std::net::TcpStream, conn: Arc<Mutex<Connection>>, data_dir: PathBuf) {
    let mut reader = BufReader::new(&stream);
    let mut request_line = String::new();
    if reader.read_line(&mut request_line).is_err() {
        return;
    }
    // Drain headers
    loop {
        let mut line = String::new();
        match reader.read_line(&mut line) {
            Ok(0) | Err(_) => break,
            _ if line == "\r\n" || line == "\n" => break,
            _ => {}
        }
    }
    let parts: Vec<&str> = request_line.split_whitespace().collect();
    if parts.len() < 2 {
        return;
    }
    match parts[1] {
        "/tracks" => serve_tracks(stream, &conn),
        p if p.starts_with("/file/") => serve_file(stream, &p[6..], &conn, &data_dir),
        _ => {
            let mut s = stream;
            let _ = s.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n");
        }
    }
}

fn serve_tracks(mut stream: std::net::TcpStream, conn: &Arc<Mutex<Connection>>) {
    let body = {
        let c = conn.lock().unwrap();
        let tracks: Vec<RemoteTrack> = c
            .prepare(
                "SELECT file_hash, path, title, artist, album \
                 FROM tracks WHERE file_hash IS NOT NULL",
            )
            .map(|mut s| {
                s.query_map([], |row| {
                    Ok(RemoteTrack {
                        hash: row.get(0)?,
                        path: row.get(1)?,
                        title: row.get(2)?,
                        artist: row.get(3)?,
                        album: row.get(4)?,
                    })
                })
                .map(|rows| rows.filter_map(|r| r.ok()).collect::<Vec<_>>())
                .unwrap_or_default()
            })
            .unwrap_or_default();
        serde_json::to_vec(&TracksResponse {
            version: APP_VERSION.to_string(),
            device_name: local_device_name(),
            tracks,
        })
        .unwrap_or_default()
    };
    let _ = write!(
        stream,
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n",
        body.len()
    );
    let _ = stream.write_all(&body);
}

fn local_device_name() -> Option<String> {
    let name = whoami::devicename();
    let trimmed = name.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn serve_file(
    mut stream: std::net::TcpStream,
    hash: &str,
    conn: &Arc<Mutex<Connection>>,
    data_dir: &Path,
) {
    let rel: Option<String> = conn
        .lock()
        .unwrap()
        .query_row(
            "SELECT path FROM tracks WHERE file_hash = ?1 LIMIT 1",
            params![hash],
            |row| row.get(0),
        )
        .ok();
    let Some(rel) = rel else {
        let mut s = stream;
        let _ = s.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n");
        return;
    };
    // Reconstruct absolute path (DB stores forward-slash relative paths).
    let abs: PathBuf = rel.split('/').fold(data_dir.to_path_buf(), |mut p, s| {
        p.push(s);
        p
    });
    match std::fs::read(&abs) {
        Ok(data) => {
            let _ = write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nContent-Length: {}\r\n\r\n",
                data.len()
            );
            let _ = stream.write_all(&data);
        }
        Err(_) => {
            let _ = stream.write_all(
                b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n",
            );
        }
    }
}

// ── Connection helpers ─────────────────────────────────────────────────────

fn connect_to_peer(peer_host: &str, peer_addresses: &[String]) -> std::io::Result<TcpStream> {
    // First try raw IP addresses from mDNS resolution, because on Android hostname
    // resolution can be flaky for .local names.
    for addr_text in peer_addresses {
        if let Ok(ip) = addr_text.parse::<IpAddr>() {
            let addr = SocketAddr::new(ip, SYNC_PORT);
            if let Ok(stream) = TcpStream::connect(addr) {
                return Ok(stream);
            }
        }
    }
    // Fallback to host resolution.
    for addr in (peer_host, SYNC_PORT).to_socket_addrs()? {
        if let Ok(stream) = TcpStream::connect(addr) {
            return Ok(stream);
        }
    }
    Err(std::io::Error::new(std::io::ErrorKind::Other, "no reachable peer address"))
}

/// Perform a blocking HTTP/1.0 GET. Returns the response body on 2xx.
fn http_get(stream: &mut TcpStream, host_header: &str, path: &str) -> std::io::Result<Vec<u8>> {
    stream.set_read_timeout(Some(Duration::from_secs(30)))?;
    write!(stream, "GET {path} HTTP/1.0\r\nHost: {host_header}\r\n\r\n")?;
    stream.flush()?;

    let mut raw = Vec::new();
    stream.read_to_end(&mut raw)?;

    // Split header / body at \r\n\r\n
    let body_start = raw
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .map(|p| p + 4)
        .unwrap_or(0);
    // Check status line ("HTTP/1.0 200 ...")
    let status_ok = raw.get(..12).map(|s| s[9..12] == *b"200").unwrap_or(false);
    if !status_ok {
        return Err(std::io::Error::new(std::io::ErrorKind::Other, "HTTP non-200"));
    }
    Ok(raw[body_start..].to_vec())
}

// ── Sync worker ───────────────────────────────────────────────────────────────

fn emit(
    app: &AppHandle,
    peer: &str,
    device_name: Option<&str>,
    phase: &str,
    total: usize,
    done: usize,
    msg: Option<String>,
) {
    let _ = app.emit(
        "sync-progress",
        SyncProgress {
            peer: peer.to_string(),
            device_name: device_name.map(|s| s.to_string()),
            phase: phase.to_string(),
            total,
            done,
            message: msg,
        },
    );
}

fn do_sync(peer_host: String, peer_name: String, peer_addresses: Vec<String>, app: AppHandle) {
    emit(&app, &peer_name, None, "index", 0, 0, None);

    let mut stream = match connect_to_peer(&peer_host, &peer_addresses) {
        Ok(s) => s,
        Err(e) => {
            emit(&app, &peer_name, None, "error", 0, 0, Some(e.to_string()));
            return;
        }
    };

    // 1 ── Fetch remote track list ─────────────────────────────────────────────
    let index_bytes = match http_get(&mut stream, &peer_host, "/tracks") {
        Ok(b) => b,
        Err(e) => {
            emit(&app, &peer_name, None, "error", 0, 0, Some(e.to_string()));
            return;
        }
    };
    let (remote_device_name, remote_tracks): (Option<String>, Vec<RemoteTrack>) =
        match serde_json::from_slice::<TracksResponse>(&index_bytes) {
            Ok(v) => (v.device_name, v.tracks),
        Err(_) => match serde_json::from_slice::<Vec<RemoteTrack>>(&index_bytes) {
            // Backward compatibility with peers that still return plain array.
            Ok(v) => (None, v),
            Err(e) => {
                emit(
                    &app,
                    &peer_name,
                    None,
                    "error",
                    0,
                    0,
                    Some(format!("parse error: {e}")),
                );
                return;
            }
        },
    };

    // 2 ── Collect local hashes (deduplication by blake3 hash) ────────────────
    let conn = app.state::<crate::library::LibraryState>().conn();
    let data_dir = app
        .state::<crate::library::LibraryState>()
        .data_dir()
        .to_path_buf();

    let local_hashes: HashSet<String> = {
        let c = conn.lock().unwrap();
        let hashes: HashSet<String> =
            match c.prepare("SELECT file_hash FROM tracks WHERE file_hash IS NOT NULL") {
                Ok(mut stmt) => stmt
                    .query_map([], |row| row.get::<_, String>(0))
                    .map(|rows| rows.filter_map(|r| r.ok()).collect())
                    .unwrap_or_default(),
                Err(_) => HashSet::new(),
            };
        hashes
    };

    // Tracks the peer has that we don't (identified solely by hash)
    let missing: Vec<RemoteTrack> = remote_tracks
        .into_iter()
        .filter(|t| !local_hashes.contains(&t.hash))
        .collect();

    let total = missing.len();
    if total == 0 {
        emit(
            &app,
            &peer_name,
            remote_device_name.as_deref(),
            "done",
            0,
            0,
            Some("Already up to date".to_string()),
        );
        return;
    }
    emit(
        &app,
        &peer_name,
        remote_device_name.as_deref(),
        "download",
        total,
        0,
        None,
    );

    // 3 ── Download missing files ──────────────────────────────────────────────
    // Saved under data_dir/Sync/<peer_name>/ preserving relative directory structure.
    let sync_root = data_dir.join("Sync").join(&peer_name);

    let mut done = 0usize;
    for track in &missing {
        // Build target path from peer's relative path
        let save_path: PathBuf = track
            .path
            .split('/')
            .fold(sync_root.clone(), |mut p, seg| {
                p.push(seg);
                p
            });

        // Guard: if a file already exists at that path, skip (e.g. re-sync after crash)
        if save_path.exists() {
            done += 1;
            emit(
                &app,
                &peer_name,
                remote_device_name.as_deref(),
                "download",
                total,
                done,
                None,
            );
            continue;
        }

        if let Some(parent) = save_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }

        let path = format!("/file/{}", track.hash);
        if let Ok(mut stream) = connect_to_peer(&peer_host, &peer_addresses) {
            if let Ok(bytes) = http_get(&mut stream, &peer_host, &path) {
                let _ = std::fs::write(&save_path, &bytes);
            }
        }

        done += 1;
        emit(
            &app,
            &peer_name,
            remote_device_name.as_deref(),
            "download",
            total,
            done,
            None,
        );
    }

    // 4 ── Reindex to register downloaded files in the DB ─────────────────────
    emit(
        &app,
        &peer_name,
        remote_device_name.as_deref(),
        "reindex",
        total,
        done,
        None,
    );
    app.state::<crate::library::LibraryState>().reindex(&app);

    emit(
        &app,
        &peer_name,
        remote_device_name.as_deref(),
        "done",
        total,
        done,
        Some(format!("{done} new track(s) added")),
    );
}

// ── Tauri commands ────────────────────────────────────────────────────────────

/// Enable or disable sync. Enabling also starts the HTTP server (once).
#[tauri::command]
pub fn sync_set_enabled(
    enabled: bool,
    state: State<'_, SyncState>,
    library: State<'_, crate::library::LibraryState>,
) -> Result<(), String> {
    let mut inner = state.inner.lock().unwrap();
    inner.enabled = enabled;
    if enabled && !inner.server_started {
        let conn = library.conn();
        let data_dir = library.data_dir().to_path_buf();
        start_http_server(conn, data_dir);
        inner.server_started = true;
    }
    Ok(())
}

/// Returns whether sync is currently enabled.
#[tauri::command]
pub fn sync_get_enabled(state: State<'_, SyncState>) -> bool {
    state.inner.lock().unwrap().enabled
}

/// Start a background sync with a specific peer.
/// Returns immediately; progress is reported via `"sync-progress"` events.
#[tauri::command]
pub fn sync_with_peer(
    peer_host: String,
    peer_name: String,
    peer_addresses: Vec<String>,
    state: State<'_, SyncState>,
    app: AppHandle,
) -> Result<(), String> {
    if !state.inner.lock().unwrap().enabled {
        return Err("Sync is disabled".to_string());
    }
    thread::spawn(move || do_sync(peer_host, peer_name, peer_addresses, app));
    Ok(())
}
