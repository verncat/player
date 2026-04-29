//! Peer sync subsystem.
//!
//! The HTTP sync server is started at app launch and stays available:
//!   • Binds a small HTTP server on SYNC_PORT (57322).
//!       GET /status        → JSON { version, device_name, playback }
//!       GET /tracks        → JSON { version, device_name, tracks:[...] }
//!       GET /file/<hash>   → raw audio bytes
//!
//! When pull sync is enabled:
//!   • `sync_with_peer` downloads every hash the peer has that we don't,
//!     skipping hash-collisions (deduplication by blake3 hash).
//!
//! Tauri event emitted to frontend: `"sync-progress"`
//!   { peer, phase: "index"|"download"|"reindex"|"done"|"error", total, done, message }

use std::collections::HashSet;
use std::io::Read;
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use rusqlite::{params, Connection};
use serde::de::DeserializeOwned;
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

/// Lightweight device state for discovery and device list UI.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct RemotePlaybackInfo {
    pub state: String,
    pub hash: Option<String>,
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub position: f64,
    pub duration: f64,
}

/// `/status` response payload for lightweight peer polling.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct DeviceStatusResponse {
    pub version: String,
    pub device_name: Option<String>,
    pub device_emoji: Option<String>,
    pub playback: Option<RemotePlaybackInfo>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct RemotePlaybackTransferRequest {
    pub hash: String,
    pub position: f64,
    pub autoplay: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct RemotePlaybackSeekRequest {
    pub position: f64,
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

// ── Sync-data payload (metadata, playlists, history) ─────────────────────────

/// Per-track metadata synced by hash.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct SyncTrackMeta {
    pub hash: String,
    pub is_liked: bool,
    pub play_count: i64,
    pub rarity: Option<String>,
    pub manually_edited: bool,
    // Fields that matter only when manually_edited
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub track_number: Option<i64>,
    pub date_added: Option<i64>,
}

/// Playlist in sync payload — tracks referenced by hash.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct SyncPlaylist {
    pub name: String,
    pub track_hashes: Vec<String>,
}

/// Smart playlist in sync payload.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct SyncSmartPlaylist {
    pub id: String,
    pub name: String,
    pub match_mode: String,
    pub rules_json: String,
    pub updated_at: i64,
}

/// Play history entry.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct SyncHistoryEntry {
    pub hash: String,
    pub played_at: i64,
}

/// Full sync-data response.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct SyncData {
    pub track_meta: Vec<SyncTrackMeta>,
    pub playlists: Vec<SyncPlaylist>,
    pub smart_playlists: Vec<SyncSmartPlaylist>,
    pub history: Vec<SyncHistoryEntry>,
}

// ── State ─────────────────────────────────────────────────────────────────────

pub struct SyncState {
    inner: Arc<Mutex<SyncInner>>,
}

struct SyncInner {
    enabled: bool,
    server_started: bool,
    in_flight_hashes: HashSet<String>,
}

impl SyncState {
    pub fn new(enabled: bool) -> Self {
        SyncState {
            inner: Arc::new(Mutex::new(SyncInner {
                enabled,
                server_started: false,
                in_flight_hashes: HashSet::new(),
            })),
        }
    }
}

pub fn ensure_http_server_started(
    state: &SyncState,
    library: &crate::library::LibraryState,
    app: &AppHandle,
) {
    let mut inner = state.inner.lock().unwrap();
    if inner.server_started {
        return;
    }
    let conn = library.conn();
    let data_dir = library.data_dir().to_path_buf();
    start_http_server(conn, data_dir, app.clone());
    inner.server_started = true;
}

// ── HTTP server (serves our tracks to peers) ──────────────────────────────────

fn start_http_server(conn: Arc<Mutex<Connection>>, data_dir: PathBuf, app: AppHandle) {
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
            let app = app.clone();
            thread::spawn(move || handle_request(request, conn, dir, app));
        }
    });
}

fn handle_request(
    request: tiny_http::Request,
    conn: Arc<Mutex<Connection>>,
    data_dir: PathBuf,
    app: AppHandle,
) {
    let url = request.url().to_string();
    if url == "/status" {
        let playback = app.state::<crate::playback::PlaybackState>();
        serve_status(request, &conn, &data_dir, &playback);
    } else if url == "/control/play-hash" {
        let playback = app.state::<crate::playback::PlaybackState>();
        serve_control_play_hash(request, &conn, &data_dir, &playback);
    } else if url == "/control/pause" {
        let playback = app.state::<crate::playback::PlaybackState>();
        serve_control_pause(request, &playback);
    } else if url == "/control/resume" {
        let playback = app.state::<crate::playback::PlaybackState>();
        serve_control_resume(request, &playback);
    } else if url == "/control/stop" {
        let playback = app.state::<crate::playback::PlaybackState>();
        serve_control_stop(request, &playback);
    } else if url == "/control/seek" {
        let playback = app.state::<crate::playback::PlaybackState>();
        serve_control_seek(request, &playback);
    } else if url == "/tracks" {
        serve_tracks(request, &conn);
    } else if url == "/sync-data" {
        serve_sync_data(request, &conn);
    } else if let Some(hash) = url.strip_prefix("/file/") {
        let hash = hash.to_string();
        serve_file(request, &hash, &conn, &data_dir);
    } else {
        let _ = request.respond(tiny_http::Response::empty(404));
    }
}

fn device_identity(conn: &Connection) -> (Option<String>, Option<String>) {
    conn.query_row(
        "SELECT COALESCE(device_name, ''), emoji FROM device_config WHERE id = 1",
        [],
        |row| {
            let name: String = row.get(0)?;
            let emoji: String = row.get(1)?;
            let name_opt = if name.trim().is_empty() { None } else { Some(name) };
            Ok((name_opt, Some(emoji)))
        },
    )
    .unwrap_or((None, None))
}

fn normalize_rel_path(path: &Path) -> String {
    path.iter()
        .map(|part| part.to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

fn rel_path_to_abs(data_dir: &Path, rel: &str) -> PathBuf {
    rel.split('/').fold(data_dir.to_path_buf(), |mut p, s| {
        p.push(s);
        p
    })
}

fn respond_status(request: tiny_http::Request, status: u16) {
    let _ = request.respond(tiny_http::Response::empty(status));
}

fn respond_json<T: Serialize>(request: tiny_http::Request, payload: &T) {
    let body = serde_json::to_vec(payload).unwrap_or_default();
    let content_type = Header::from_bytes(b"Content-Type", b"application/json").unwrap();
    let _ = request.respond(tiny_http::Response::from_data(body).with_header(content_type));
}

fn respond_error(request: tiny_http::Request, status: u16, message: &str) {
    let content_type = Header::from_bytes(b"Content-Type", b"text/plain; charset=utf-8").unwrap();
    let _ = request.respond(
        tiny_http::Response::from_string(message.to_string())
            .with_status_code(status)
            .with_header(content_type),
    );
}

fn read_json_request<T: DeserializeOwned>(request: &mut tiny_http::Request) -> Result<T, String> {
    let mut body = Vec::new();
    request
        .as_reader()
        .read_to_end(&mut body)
        .map_err(|e| e.to_string())?;
    serde_json::from_slice(&body).map_err(|e| e.to_string())
}

fn current_playback_info(
    conn: &Connection,
    data_dir: &Path,
    playback: &crate::playback::PlaybackState,
) -> Option<RemotePlaybackInfo> {
    let current_file = playback.current_file_path()?;
    let rel_path = current_file
        .strip_prefix(data_dir)
        .ok()
        .map(normalize_rel_path);
    let metadata = rel_path.as_ref().and_then(|rel| {
        conn.query_row(
            "SELECT file_hash, title, artist, album FROM tracks WHERE path = ?1 LIMIT 1",
            params![rel],
            |row| {
                Ok((
                    row.get::<_, Option<String>>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, Option<String>>(3)?,
                ))
            },
        )
        .ok()
    });
    let fallback_title = current_file
        .file_stem()
        .map(|stem| stem.to_string_lossy().to_string())
        .filter(|value| !value.trim().is_empty());
    let (hash, title, artist, album) = metadata.unwrap_or((None, fallback_title, None, None));
    let state = if playback.is_playing() {
        "playing"
    } else if playback.is_finished() {
        "ended"
    } else if playback.is_stopped() {
        "stopped"
    } else {
        "paused"
    };
    Some(RemotePlaybackInfo {
        state: state.to_string(),
        hash,
        title,
        artist,
        album,
        position: playback.position_secs(),
        duration: playback.duration_secs(),
    })
}

fn serve_status(
    request: tiny_http::Request,
    conn: &Arc<Mutex<Connection>>,
    data_dir: &Path,
    playback: &crate::playback::PlaybackState,
) {
    let body = {
        let c = conn.lock().unwrap();
        let status = build_device_status(&c, data_dir, playback);
        serde_json::to_vec(&status).unwrap_or_default()
    };
    let content_type = Header::from_bytes(b"Content-Type", b"application/json").unwrap();
    let _ = request.respond(tiny_http::Response::from_data(body).with_header(content_type));
}

fn build_device_status(
    conn: &Connection,
    data_dir: &Path,
    playback: &crate::playback::PlaybackState,
) -> DeviceStatusResponse {
    let (device_name, device_emoji) = device_identity(conn);
    let playback = current_playback_info(conn, data_dir, playback);
    DeviceStatusResponse {
        version: APP_VERSION.to_string(),
        device_name,
        device_emoji,
        playback,
    }
}

fn serve_control_play_hash(
    mut request: tiny_http::Request,
    conn: &Arc<Mutex<Connection>>,
    data_dir: &Path,
    playback: &crate::playback::PlaybackState,
) {
    let payload = match read_json_request::<RemotePlaybackTransferRequest>(&mut request) {
        Ok(payload) => payload,
        Err(e) => {
            respond_error(request, 400, &format!("invalid request: {e}"));
            return;
        }
    };
    let rel: Option<String> = conn
        .lock()
        .unwrap()
        .query_row(
            "SELECT path FROM tracks WHERE file_hash = ?1 LIMIT 1",
            params![payload.hash],
            |row| row.get(0),
        )
        .ok();
    let Some(rel) = rel else {
        respond_error(request, 404, "track not found");
        return;
    };
    let abs = rel_path_to_abs(data_dir, &rel);
    if let Err(e) = playback.play(abs) {
        respond_error(request, 500, &e);
        return;
    }
    if payload.position.is_finite() && payload.position > 0.0 {
        playback.seek(payload.position.max(0.0));
    }
    if !payload.autoplay {
        playback.pause();
    }
    respond_status(request, 204);
}

fn serve_control_pause(request: tiny_http::Request, playback: &crate::playback::PlaybackState) {
    playback.pause();
    respond_status(request, 204);
}

fn serve_control_resume(request: tiny_http::Request, playback: &crate::playback::PlaybackState) {
    playback.resume();
    respond_status(request, 204);
}

fn serve_control_stop(request: tiny_http::Request, playback: &crate::playback::PlaybackState) {
    playback.stop();
    respond_status(request, 204);
}

fn serve_control_seek(mut request: tiny_http::Request, playback: &crate::playback::PlaybackState) {
    let payload = match read_json_request::<RemotePlaybackSeekRequest>(&mut request) {
        Ok(payload) => payload,
        Err(e) => {
            respond_error(request, 400, &format!("invalid request: {e}"));
            return;
        }
    };
    playback.seek(payload.position.max(0.0));
    respond_status(request, 204);
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
        let (device_name, device_emoji) = device_identity(&c);
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

fn serve_sync_data(request: tiny_http::Request, conn: &Arc<Mutex<Connection>>) {
    let c = conn.lock().unwrap();

    // 1. Track metadata
    let track_meta: Vec<SyncTrackMeta> = c
        .prepare(
            "SELECT file_hash, is_liked, play_count, rarity, manually_edited,
                    title, artist, album, track_number, date_added
             FROM tracks WHERE file_hash IS NOT NULL",
        )
        .map(|mut s| {
            s.query_map([], |row| {
                Ok(SyncTrackMeta {
                    hash: row.get(0)?,
                    is_liked: row.get::<_, i64>(1).unwrap_or(0) != 0,
                    play_count: row.get::<_, i64>(2).unwrap_or(0),
                    rarity: row.get(3)?,
                    manually_edited: row.get::<_, i64>(4).unwrap_or(0) != 0,
                    title: row.get(5)?,
                    artist: row.get(6)?,
                    album: row.get(7)?,
                    track_number: row.get(8)?,
                    date_added: row.get(9)?,
                })
            })
            .map(|rows| rows.filter_map(|r| r.ok()).collect::<Vec<_>>())
            .unwrap_or_default()
        })
        .unwrap_or_default();

    // 2. Regular playlists (name + track hashes in order)
    let playlists: Vec<SyncPlaylist> = c
        .prepare("SELECT id, name FROM playlists ORDER BY created_at")
        .map(|mut s| {
            s.query_map([], |row| {
                let pid: i64 = row.get(0)?;
                let name: String = row.get(1)?;
                Ok((pid, name))
            })
            .map(|rows| rows.filter_map(|r| r.ok()).collect::<Vec<_>>())
            .unwrap_or_default()
        })
        .unwrap_or_default()
        .into_iter()
        .map(|(pid, name)| {
            let hashes: Vec<String> = c
                .prepare(
                    "SELECT t.file_hash FROM playlist_tracks pt
                     JOIN tracks t ON t.id = pt.track_id
                     WHERE pt.playlist_id = ?1 AND t.file_hash IS NOT NULL
                     ORDER BY pt.position, pt.id",
                )
                .and_then(|mut s| {
                    s.query_map(params![pid], |row| row.get::<_, String>(0))
                        .map(|rows| rows.filter_map(|r| r.ok()).collect())
                })
                .unwrap_or_default();
            SyncPlaylist { name, track_hashes: hashes }
        })
        .collect();

    // 3. Smart playlists
    let smart_playlists: Vec<SyncSmartPlaylist> = c
        .prepare("SELECT id, name, match_mode, rules_json, updated_at FROM smart_playlists")
        .map(|mut s| {
            s.query_map([], |row| {
                Ok(SyncSmartPlaylist {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    match_mode: row.get(2)?,
                    rules_json: row.get(3)?,
                    updated_at: row.get(4)?,
                })
            })
            .map(|rows| rows.filter_map(|r| r.ok()).collect::<Vec<_>>())
            .unwrap_or_default()
        })
        .unwrap_or_default();

    // 4. Play history (hash + timestamp)
    let history: Vec<SyncHistoryEntry> = c
        .prepare(
            "SELECT t.file_hash, ph.played_at
             FROM play_history ph
             JOIN tracks t ON t.id = ph.track_id
             WHERE t.file_hash IS NOT NULL
             ORDER BY ph.played_at",
        )
        .map(|mut s| {
            s.query_map([], |row| {
                Ok(SyncHistoryEntry {
                    hash: row.get(0)?,
                    played_at: row.get(1)?,
                })
            })
            .map(|rows| rows.filter_map(|r| r.ok()).collect::<Vec<_>>())
            .unwrap_or_default()
        })
        .unwrap_or_default();

    drop(c);

    let body = serde_json::to_vec(&SyncData {
        track_meta,
        playlists,
        smart_playlists,
        history,
    })
    .unwrap_or_default();
    let ct = Header::from_bytes(b"Content-Type", b"application/json").unwrap();
    let _ = request.respond(tiny_http::Response::from_data(body).with_header(ct));
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

fn peer_post_json<T: Serialize>(
    client: &reqwest::blocking::Client,
    base_urls: &[String],
    path: &str,
    payload: &T,
) -> Result<(), String> {
    let mut last_err = String::from("no reachable peer address");
    for base in base_urls {
        let url = format!("{}{}", base, path);
        match client.post(&url).json(payload).send() {
            Ok(resp) if resp.status().is_success() => return Ok(()),
            Ok(resp) => {
                let status = resp.status();
                let body = resp.text().unwrap_or_default();
                last_err = if body.trim().is_empty() {
                    format!("HTTP {}", status)
                } else {
                    format!("HTTP {}: {}", status, body)
                };
            }
            Err(e) => {
                last_err = e.to_string();
            }
        }
    }
    Err(last_err)
}

fn peer_post_empty(
    client: &reqwest::blocking::Client,
    base_urls: &[String],
    path: &str,
) -> Result<(), String> {
    let mut last_err = String::from("no reachable peer address");
    for base in base_urls {
        let url = format!("{}{}", base, path);
        match client.post(&url).send() {
            Ok(resp) if resp.status().is_success() => return Ok(()),
            Ok(resp) => {
                let status = resp.status();
                let body = resp.text().unwrap_or_default();
                last_err = if body.trim().is_empty() {
                    format!("HTTP {}", status)
                } else {
                    format!("HTTP {}: {}", status, body)
                };
            }
            Err(e) => {
                last_err = e.to_string();
            }
        }
    }
    Err(last_err)
}

fn build_remote_client(timeout: Duration) -> Result<reqwest::blocking::Client, String> {
    reqwest::blocking::Client::builder()
        .timeout(timeout)
        .build()
        .map_err(|e| e.to_string())
}

fn remote_status_via_http(base_urls: &[String]) -> Result<DeviceStatusResponse, String> {
    let client = build_remote_client(Duration::from_secs(5))?;
    let bytes = peer_get(&client, base_urls, "/status")?;
    serde_json::from_slice::<DeviceStatusResponse>(&bytes).map_err(|e| e.to_string())
}

fn remote_control_post<T: Serialize>(
    base_urls: &[String],
    path: &str,
    payload: &T,
) -> Result<(), String> {
    let client = build_remote_client(Duration::from_secs(10))?;
    peer_post_json(&client, base_urls, path, payload)
}

fn remote_control_post_empty(base_urls: &[String], path: &str) -> Result<(), String> {
    let client = build_remote_client(Duration::from_secs(10))?;
    peer_post_empty(&client, base_urls, path)
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

fn local_hash_exists(conn: &Connection, hash: &str) -> bool {
    conn.query_row(
        "SELECT 1 FROM tracks WHERE file_hash = ?1 LIMIT 1",
        params![hash],
        |_| Ok(()),
    )
    .is_ok()
}

fn claim_in_flight_hash(app: &AppHandle, conn: &Arc<Mutex<Connection>>, hash: &str) -> bool {
    let c = conn.lock().unwrap();
    if local_hash_exists(&c, hash) {
        return false;
    }

    let sync = app.state::<SyncState>();
    let mut inner = sync.inner.lock().unwrap();
    if inner.in_flight_hashes.contains(hash) {
        return false;
    }

    inner.in_flight_hashes.insert(hash.to_string());
    true
}

fn release_in_flight_hash(app: &AppHandle, hash: &str) {
    let sync = app.state::<SyncState>();
    let mut inner = sync.inner.lock().unwrap();
    inner.in_flight_hashes.remove(hash);
}

fn register_downloaded_hash(
    conn: &Arc<Mutex<Connection>>,
    data_dir: &Path,
    abs_path: &Path,
    hash: &str,
) -> Result<(), String> {
    let rel = abs_path
        .strip_prefix(data_dir)
        .map_err(|_| {
            format!(
                "downloaded file {} is outside data dir {}",
                abs_path.display(),
                data_dir.display()
            )
        })?
        .to_string_lossy()
        .replace('\\', "/");
    let now_secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;

    conn.lock()
        .unwrap()
        .execute(
            "INSERT INTO tracks (path, modified_secs, file_hash, date_added)
             VALUES (?1, 0, ?2, ?3)
             ON CONFLICT(path) DO UPDATE SET file_hash = excluded.file_hash",
            params![rel, hash, now_secs],
        )
        .map_err(|e| e.to_string())?;

    Ok(())
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
    let mut added = 0usize;
    for track in &missing {
        // Build target path from peer's relative path
        let save_path: PathBuf = track
            .path
            .split('/')
            .fold(sync_root.clone(), |mut p, seg| {
                p.push(seg);
                p
            });

        // Recheck the DB at download time and reserve this hash across
        // concurrent sync workers so two peers can't fetch the same track.
        if !claim_in_flight_hash(&app, &conn, &track.hash) {
            done += 1;
            let label = track.title.as_deref()
                .filter(|t| !t.is_empty())
                .unwrap_or_else(|| track.path.rsplit('/').next().unwrap_or(track.path.as_str()));
            emit(
                &app,
                &peer_name,
                remote_device_name.as_deref(),
                remote_device_emoji.as_deref(),
                "download",
                total,
                done,
                Some(label.to_string()),
            );
            continue;
        }

        // Guard: if a file already exists at that path, skip (e.g. re-sync after crash)
        if save_path.exists() {
            if let Err(e) = register_downloaded_hash(&conn, &data_dir, &save_path, &track.hash) {
                eprintln!("[sync] failed to register existing file for hash {}: {e}", track.hash);
            } else {
                added += 1;
            }
            release_in_flight_hash(&app, &track.hash);
            done += 1;
            let label = track.title.as_deref()
                .filter(|t| !t.is_empty())
                .unwrap_or_else(|| track.path.rsplit('/').next().unwrap_or(track.path.as_str()));
            emit(
                &app,
                &peer_name,
                remote_device_name.as_deref(),
                remote_device_emoji.as_deref(),
                "download",
                total,
                done,
                Some(label.to_string()),
            );
            continue;
        }

        if let Some(parent) = save_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }

        let path = format!("/file/{}", track.hash);
        if let Ok(bytes) = peer_get(&client, &base_urls, &path) {
            if std::fs::write(&save_path, &bytes).is_ok() {
                if let Err(e) = register_downloaded_hash(&conn, &data_dir, &save_path, &track.hash) {
                    eprintln!("[sync] failed to register downloaded file for hash {}: {e}", track.hash);
                } else {
                    added += 1;
                }
            }
        }
        release_in_flight_hash(&app, &track.hash);

        done += 1;
        let label = track.title.as_deref()
            .filter(|t| !t.is_empty())
            .unwrap_or_else(|| track.path.rsplit('/').next().unwrap_or(track.path.as_str()));
        emit(
            &app,
            &peer_name,
            remote_device_name.as_deref(),
            remote_device_emoji.as_deref(),
            "download",
            total,
            done,
            Some(label.to_string()),
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

    // 5 ── Sync metadata, playlists, smart playlists, history ─────────────────
    emit(
        &app,
        &peer_name,
        remote_device_name.as_deref(),
        remote_device_emoji.as_deref(),
        "merging",
        total,
        done,
        Some("Merging metadata…".to_string()),
    );
    if let Ok(sd_bytes) = peer_get(&client, &base_urls, "/sync-data") {
        if let Ok(remote) = serde_json::from_slice::<SyncData>(&sd_bytes) {
            merge_sync_data(&app, remote);
        }
    }

    emit(
        &app,
        &peer_name,
        remote_device_name.as_deref(),
        remote_device_emoji.as_deref(),
        "done",
        total,
        done,
        Some(format!("{added} new track(s) added")),
    );
}

// ── Merge helpers ─────────────────────────────────────────────────────────────

fn merge_sync_data(app: &AppHandle, remote: SyncData) {
    let conn = app.state::<crate::library::LibraryState>().conn();
    let c = conn.lock().unwrap();

    // Build hash → local track id map
    let mut hash_to_id: std::collections::HashMap<String, i64> = std::collections::HashMap::new();
    if let Ok(mut stmt) = c.prepare("SELECT file_hash, id FROM tracks WHERE file_hash IS NOT NULL") {
        if let Ok(rows) = stmt.query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))) {
            for r in rows.flatten() {
                hash_to_id.insert(r.0, r.1);
            }
        }
    }

    // ── Track metadata merge ────────────────────────────────────────────────
    // is_liked: OR (liked anywhere → liked everywhere)
    // play_count: MAX (take whichever is higher)
    // rarity: keep remote if local is NULL
    // manually_edited metadata: if remote is manually_edited and local is not, adopt remote edits
    for tm in &remote.track_meta {
        let Some(&tid) = hash_to_id.get(&tm.hash) else { continue };

        // Read current local state for this track
        let local: Option<(bool, i64, Option<String>, bool, Option<i64>)> = c
            .query_row(
                "SELECT is_liked, play_count, rarity, manually_edited, date_added FROM tracks WHERE id = ?1",
                params![tid],
                |row| Ok((
                    row.get::<_, i64>(0).unwrap_or(0) != 0,
                    row.get::<_, i64>(1).unwrap_or(0),
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, i64>(3).unwrap_or(0) != 0,
                    row.get::<_, Option<i64>>(4)?,
                )),
            )
            .ok();
        let Some((local_liked, local_pc, local_rarity, local_edited, local_date_added)) = local else { continue };

        let merged_liked = local_liked || tm.is_liked;
        let merged_pc = local_pc.max(tm.play_count);
        let merged_rarity = local_rarity.or_else(|| tm.rarity.clone());
        // Keep earliest date_added (the original local indexation date wins)
        let merged_date_added = match (local_date_added, tm.date_added) {
            (Some(l), Some(r)) => Some(l.min(r)),
            (Some(l), None) => Some(l),
            (None, r) => r,
        };

        let _ = c.execute(
            "UPDATE tracks SET is_liked = ?1, play_count = ?2, rarity = ?3, date_added = COALESCE(?5, date_added) WHERE id = ?4",
            params![merged_liked as i64, merged_pc, merged_rarity, tid, merged_date_added],
        );

        // If remote has manual edits and we don't, adopt them
        if tm.manually_edited && !local_edited {
            let _ = c.execute(
                "UPDATE tracks SET title = ?1, artist = ?2, album = ?3, track_number = ?4, manually_edited = 1 WHERE id = ?5",
                params![tm.title, tm.artist, tm.album, tm.track_number, tid],
            );
        }
    }

    // ── Playlist merge ──────────────────────────────────────────────────────
    // By name: if playlist exists locally, union track hashes; if not, create it.
    for rp in &remote.playlists {
        let existing_id: Option<i64> = c
            .query_row("SELECT id FROM playlists WHERE name = ?1", params![rp.name], |row| row.get(0))
            .ok();
        let pid = match existing_id {
            Some(id) => id,
            None => {
                let _ = c.execute("INSERT INTO playlists (name) VALUES (?1)", params![rp.name]);
                c.last_insert_rowid()
            }
        };
        // Get current max position
        let max_pos: i64 = c
            .query_row(
                "SELECT COALESCE(MAX(position), -1) FROM playlist_tracks WHERE playlist_id = ?1",
                params![pid],
                |row| row.get(0),
            )
            .unwrap_or(-1);
        let mut pos = max_pos + 1;
        for hash in &rp.track_hashes {
            if let Some(&tid) = hash_to_id.get(hash) {
                // Insert only if not already in this playlist
                let r = c.execute(
                    "INSERT OR IGNORE INTO playlist_tracks (playlist_id, track_id, position) VALUES (?1, ?2, ?3)",
                    params![pid, tid, pos],
                );
                if r.map(|n| n > 0).unwrap_or(false) {
                    pos += 1;
                }
            }
        }
    }

    // ── Smart playlist merge ────────────────────────────────────────────────
    // By UUID: if same id exists, take whichever has higher updated_at.
    // If id doesn't exist, insert.
    for rsp in &remote.smart_playlists {
        let local_updated: Option<i64> = c
            .query_row("SELECT updated_at FROM smart_playlists WHERE id = ?1", params![rsp.id], |row| row.get(0))
            .ok();
        match local_updated {
            Some(lu) if lu >= rsp.updated_at => {} // local is newer, skip
            _ => {
                // Remote is newer or doesn't exist locally — upsert
                let _ = c.execute(
                    "INSERT INTO smart_playlists (id, name, match_mode, rules_json, updated_at)
                     VALUES (?1, ?2, ?3, ?4, ?5)
                     ON CONFLICT(id) DO UPDATE SET
                       name = excluded.name,
                       match_mode = excluded.match_mode,
                       rules_json = excluded.rules_json,
                       updated_at = excluded.updated_at",
                    params![rsp.id, rsp.name, rsp.match_mode, rsp.rules_json, rsp.updated_at],
                );
            }
        }
    }

    // ── Play history merge ──────────────────────────────────────────────────
    // Union: insert (track_id, played_at) pairs that don't exist yet.
    // Dedup by exact (track, timestamp) — same second = same event.
    for rh in &remote.history {
        let Some(&tid) = hash_to_id.get(&rh.hash) else { continue };
        let exists: bool = c
            .query_row(
                "SELECT 1 FROM play_history WHERE track_id = ?1 AND played_at = ?2",
                params![tid, rh.played_at],
                |_| Ok(()),
            )
            .is_ok();
        if !exists {
            let _ = c.execute(
                "INSERT INTO play_history (track_id, played_at) VALUES (?1, ?2)",
                params![tid, rh.played_at],
            );
        }
    }
}

// ── Tauri commands ────────────────────────────────────────────────────────────

/// Enable or disable pull sync. The HTTP server is started separately and kept available.
#[tauri::command]
pub fn sync_set_enabled(
    enabled: bool,
    state: State<'_, SyncState>,
    library: State<'_, crate::library::LibraryState>,
    app: AppHandle,
) -> Result<(), String> {
    ensure_http_server_started(&state, &library, &app);
    let current = library.get_device_settings().map_err(|e| e.to_string())?;
    library
        .set_device_settings(
            &current.emoji,
            &current.device_name,
            enabled,
            current.soulseek_enabled,
            &current.soulseek_username,
            &current.soulseek_password,
        )
        .map_err(|e| e.to_string())?;
    let mut inner = state.inner.lock().unwrap();
    inner.enabled = enabled;
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

#[tauri::command]
pub fn remote_playback_status(
    peer_host: String,
    peer_addresses: Vec<String>,
    peer_port: Option<u16>,
) -> Result<DeviceStatusResponse, String> {
    let base_urls = peer_base_urls(&peer_host, &peer_addresses, peer_port.unwrap_or(SYNC_PORT));
    remote_status_via_http(&base_urls)
}

#[tauri::command]
pub fn remote_playback_transfer(
    peer_host: String,
    peer_addresses: Vec<String>,
    peer_port: Option<u16>,
    hash: String,
    position: f64,
    autoplay: bool,
) -> Result<(), String> {
    let base_urls = peer_base_urls(&peer_host, &peer_addresses, peer_port.unwrap_or(SYNC_PORT));
    remote_control_post(
        &base_urls,
        "/control/play-hash",
        &RemotePlaybackTransferRequest { hash, position, autoplay },
    )
}

#[tauri::command]
pub fn remote_playback_pause(
    peer_host: String,
    peer_addresses: Vec<String>,
    peer_port: Option<u16>,
) -> Result<(), String> {
    let base_urls = peer_base_urls(&peer_host, &peer_addresses, peer_port.unwrap_or(SYNC_PORT));
    remote_control_post_empty(&base_urls, "/control/pause")
}

#[tauri::command]
pub fn remote_playback_resume(
    peer_host: String,
    peer_addresses: Vec<String>,
    peer_port: Option<u16>,
) -> Result<(), String> {
    let base_urls = peer_base_urls(&peer_host, &peer_addresses, peer_port.unwrap_or(SYNC_PORT));
    remote_control_post_empty(&base_urls, "/control/resume")
}

#[tauri::command]
pub fn remote_playback_stop(
    peer_host: String,
    peer_addresses: Vec<String>,
    peer_port: Option<u16>,
) -> Result<(), String> {
    let base_urls = peer_base_urls(&peer_host, &peer_addresses, peer_port.unwrap_or(SYNC_PORT));
    remote_control_post_empty(&base_urls, "/control/stop")
}

#[tauri::command]
pub fn remote_playback_seek(
    peer_host: String,
    peer_addresses: Vec<String>,
    peer_port: Option<u16>,
    position: f64,
) -> Result<(), String> {
    let base_urls = peer_base_urls(&peer_host, &peer_addresses, peer_port.unwrap_or(SYNC_PORT));
    remote_control_post(
        &base_urls,
        "/control/seek",
        &RemotePlaybackSeekRequest { position },
    )
}
