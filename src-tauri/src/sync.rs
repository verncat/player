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
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, State};
use tiny_http::Header;

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
    pub device_emoji: Option<String>,
    pub tracks: Vec<RemoteTrack>,
}

/// Progress event payload.
#[derive(Serialize, Clone, Debug)]
pub struct SyncProgress {
    pub peer: String,
    pub device_name: Option<String>,
    pub device_emoji: Option<String>,
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
        let server = match tiny_http::Server::http(format!("[::]:{SYNC_PORT}"))
            .or_else(|_| tiny_http::Server::http(format!("0.0.0.0:{SYNC_PORT}")))
        {
            Ok(s) => s,
            Err(e) => {
                eprintln!("[sync] cannot bind port {SYNC_PORT}: {e}");
                return;
            }
        };
        eprintln!("[sync] HTTP server ready on :{SYNC_PORT}");
        for request in server.incoming_requests() {
            let conn = Arc::clone(&conn);
            let dir = data_dir.clone();
            thread::spawn(move || handle_request(request, conn, dir));
        }
    });
}

fn handle_request(request: tiny_http::Request, conn: Arc<Mutex<Connection>>, data_dir: PathBuf) {
    let url = request.url().to_string();
    if url == "/tracks" {
        serve_tracks(request, &conn);
    } else if let Some(hash) = url.strip_prefix("/file/") {
        let hash = hash.to_string();
        serve_file(request, &hash, &conn, &data_dir);
    } else {
        let _ = request.respond(tiny_http::Response::empty(404));
    }
}

fn serve_tracks(request: tiny_http::Request, conn: &Arc<Mutex<Connection>>) {
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
        let (device_name, device_emoji) = c
            .query_row(
                "SELECT COALESCE(device_name, ''), emoji FROM device_config WHERE id = 1",
                [],
                |row| {
                    let name: String = row.get(0)?;
                    let emoji: String = row.get(1)?;
                    let name_opt = if name.trim().is_empty() { None } else { Some(name) };
                    Ok((name_opt, Some(emoji)))
                },
            )
            .unwrap_or((None, None));
        serde_json::to_vec(&TracksResponse {
            version: APP_VERSION.to_string(),
            device_name,
            device_emoji,
            tracks,
        })
        .unwrap_or_default()
    };
    let content_type = Header::from_bytes(b"Content-Type", b"application/json").unwrap();
    let _ = request.respond(tiny_http::Response::from_data(body).with_header(content_type));
}

fn serve_file(request: tiny_http::Request, hash: &str, conn: &Arc<Mutex<Connection>>, data_dir: &Path) {
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
        let _ = request.respond(tiny_http::Response::empty(404));
        return;
    };
    let abs: PathBuf = rel.split('/').fold(data_dir.to_path_buf(), |mut p, s| {
        p.push(s);
        p
    });
    match std::fs::read(&abs) {
        Ok(data) => {
            let content_type =
                Header::from_bytes(b"Content-Type", b"application/octet-stream").unwrap();
            let _ = request
                .respond(tiny_http::Response::from_data(data).with_header(content_type));
        }
        Err(_) => {
            let _ = request.respond(tiny_http::Response::empty(404));
        }
    }
}

// ── HTTP client helpers ───────────────────────────────────────────────────────

/// Build the list of URLs to try for a peer, in priority order.
fn peer_base_urls(peer_host: &str, peer_addresses: &[String], port: u16) -> Vec<String> {
    // If peer_host is already a resolved IP (set by best_addr during discovery),
    // use it directly — no need to iterate peer_addresses as fallback.
    if let Ok(ip) = peer_host.parse::<IpAddr>() {
        let formatted = if matches!(ip, IpAddr::V6(_)) {
            format!("http://[{}]:{}", peer_host, port)
        } else {
            format!("http://{}:{}", peer_host, port)
        };
        return vec![formatted];
    }
    // peer_host is a hostname (.local etc.) — try it first, then fall back to
    // raw IPs from mDNS in case hostname resolution is broken on this platform.
    let mut urls = vec![format!("http://{}:{}", peer_host, port)];
    for addr_str in peer_addresses {
        if let Ok(ip) = addr_str.parse::<IpAddr>() {
            // Skip link-local IPv6 — no scope-id available.
            if let IpAddr::V6(v6) = ip {
                if v6.is_unicast_link_local() {
                    continue;
                }
                urls.push(format!("http://[{}]:{}", ip, port));
            } else {
                urls.push(format!("http://{}:{}", ip, port));
            }
        }
    }
    urls
}

/// GET `path` from the first reachable peer URL. Returns the response body on 2xx.
fn peer_get(client: &reqwest::blocking::Client, base_urls: &[String], path: &str) -> Result<Vec<u8>, String> {
    let mut last_err = String::from("no reachable peer address");
    for base in base_urls {
        let url = format!("{}{}", base, path);
        match client.get(&url).send() {
            Ok(resp) if resp.status().is_success() => {
                return resp.bytes().map(|b| b.to_vec()).map_err(|e| e.to_string());
            }
            Ok(resp) => {
                last_err = format!("HTTP {}", resp.status());
            }
            Err(e) => {
                last_err = e.to_string();
            }
        }
    }
    Err(last_err)
}
// ── Sync worker ───────────────────────────────────────────────────────────────

fn emit(
    app: &AppHandle,
    peer: &str,
    device_name: Option<&str>,
    device_emoji: Option<&str>,
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
            device_emoji: device_emoji.map(|s| s.to_string()),
            phase: phase.to_string(),
            total,
            done,
            message: msg,
        },
    );
}

fn do_sync(peer_host: String, peer_name: String, peer_addresses: Vec<String>, peer_port: u16, app: AppHandle) {
    emit(&app, &peer_name, None, None, "index", 0, 0, Some("Connecting...".to_string()));

    let client = match reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            emit(&app, &peer_name, None, None, "error", 0, 0, Some(e.to_string()));
            return;
        }
    };
    let base_urls = peer_base_urls(&peer_host, &peer_addresses, peer_port);

    // 1 ── Fetch remote track list ───────────────────────────────────────────────────
    emit(&app, &peer_name, None, None, "index", 0, 0, Some("Fetching index...".to_string()));
    let index_bytes = match peer_get(&client, &base_urls, "/tracks") {
        Ok(b) => b,
        Err(e) => {
            emit(&app, &peer_name, None, None, "error", 0, 0, Some(e.to_string()));
            return;
        }
    };
    let (remote_device_name, remote_device_emoji, remote_tracks): (Option<String>, Option<String>, Vec<RemoteTrack>) =
        match serde_json::from_slice::<TracksResponse>(&index_bytes) {
            Ok(v) => (v.device_name, v.device_emoji, v.tracks),
        Err(_) => match serde_json::from_slice::<Vec<RemoteTrack>>(&index_bytes) {
            // Backward compatibility with peers that still return plain array.
            Ok(v) => (None, None, v),
            Err(e) => {
                emit(
                    &app,
                    &peer_name,
                    None,
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
    emit(
        &app,
        &peer_name,
        remote_device_name.as_deref(),
        remote_device_emoji.as_deref(),
        "index",
        0,
        0,
        Some("Comparing libraries...".to_string()),
    );
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
            remote_device_emoji.as_deref(),
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
        remote_device_emoji.as_deref(),
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
                remote_device_emoji.as_deref(),
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
        if let Ok(bytes) = peer_get(&client, &base_urls, &path) {
            let _ = std::fs::write(&save_path, &bytes);
        }

        done += 1;
        emit(
            &app,
            &peer_name,
            remote_device_name.as_deref(),
            remote_device_emoji.as_deref(),
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
        remote_device_emoji.as_deref(),
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
        remote_device_emoji.as_deref(),
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
    peer_port: Option<u16>,
    state: State<'_, SyncState>,
    app: AppHandle,
) -> Result<(), String> {
    if !state.inner.lock().unwrap().enabled {
        return Err("Sync is disabled".to_string());
    }
    let port = peer_port.unwrap_or(SYNC_PORT);
    thread::spawn(move || do_sync(peer_host, peer_name, peer_addresses, port, app));
    Ok(())
}
