//! Peer sync subsystem.
//!
//! The HTTP sync server is started at app launch and stays available:
//!   • Binds a small HTTP server on SYNC_PORT (57322).
//!       GET /status        → JSON { version, device_name, playback }
//!       GET /tracks        → JSON { version, device_name, tracks:[...] }
//!       GET /file/<hash>   → raw audio bytes
//!       GET /sync-merkle/* → Merkle summaries and selective sync leaves
//!       GET /sync-merkle/debug → compact hash/count diagnostics
//!
//! When pull sync is enabled:
//!   • `sync_with_peer` downloads every hash the peer has that we don't,
//!     skipping hash-collisions (deduplication by blake3 hash).
//!
//! Tauri event emitted to frontend: `"sync-progress"`
//!   { peer, phase: "index"|"download"|"reindex"|"done"|"error", total, done, message }

use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::Read;
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use rusqlite::{params, Connection};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, State};
use tiny_http::Header;

pub const SYNC_PORT: u16 = 57322;
pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");
const SYNC_MERKLE_VERSION: &str = "1";
const PLAY_HISTORY_CHUNK_SECONDS: i64 = 24 * 60 * 60;

// ── Public types ──────────────────────────────────────────────────────────────

/// Track info exchanged with peers.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct RemoteTrack {
    pub hash: String,
    pub path: String,
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub source_kind: Option<String>,
    pub source_path: Option<String>,
    pub cue_path: Option<String>,
    pub file_size: Option<u64>,
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
    pub item_done: Option<u64>,
    pub item_total: Option<u64>,
}

// ── Sync-data contract (track fields, playlists, smart playlists, history) ──

/// Per-track metadata synced by hash.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct SyncTrackFields {
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub track_number: Option<i64>,
    pub year: Option<i64>,
    pub genre: Option<String>,
    pub tags: Option<String>,
    pub date_added: Option<i64>,
}

/// Per-track metadata synced by hash.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct SyncTrackMeta {
    pub hash: String,
    pub is_liked: bool,
    pub play_count: i64,
    pub rarity: Option<String>,
    pub manually_edited: bool,
    // Fields that matter only when manually_edited.
    #[serde(flatten, default)]
    pub fields: SyncTrackFields,
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

/// Per-track mutable state synced independently from editable fields.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct SyncTrackState {
    pub is_liked: bool,
    pub play_count: i64,
    pub rarity: Option<String>,
    pub manually_edited: bool,
}

/// Per-track Merkle child hashes.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct SyncTrackMetaHashes {
    pub hash: String,
    pub state_hash: String,
    pub fields_hash: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct SyncTrackStatePayload {
    pub hash: String,
    pub state: SyncTrackState,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct SyncTrackFieldsPayload {
    pub hash: String,
    pub fields: SyncTrackFields,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct SyncContentHashes {
    pub hash: String,
    pub blob_hash: String,
    pub descriptor_hash: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct SyncPlaylistHashes {
    pub name: String,
    pub identity_hash: String,
    pub tracks_hash: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct SyncSmartPlaylistHashes {
    pub id: String,
    pub node_hash: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct SyncHistoryChunkRange {
    pub min_played_at: i64,
    pub max_played_at: i64,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct SyncHistoryChunkSummary {
    pub chunk_id: String,
    pub range: SyncHistoryChunkRange,
    pub event_count: usize,
    pub rolling_hash: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct SyncHistoryChunk {
    pub chunk_id: String,
    pub range: SyncHistoryChunkRange,
    pub event_count: usize,
    pub rolling_hash: String,
    pub events: Vec<SyncHistoryEntry>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct SyncMerkleRoot {
    pub version: String,
    pub root_hash: String,
    pub content_by_hash_hash: String,
    pub library_state_hash: String,
    pub track_meta_by_hash_hash: String,
    pub playlists_by_name_hash: String,
    pub smart_playlists_by_id_hash: String,
    pub play_history_log_hash: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct SyncMerkleDebugCounts {
    pub content_entries: usize,
    pub track_meta_entries: usize,
    pub playlists: usize,
    pub smart_playlists: usize,
    pub history_chunks: usize,
    pub history_events_total: usize,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct SyncMerkleDebugResponse {
    pub app_version: String,
    pub merkle_version: String,
    pub root: SyncMerkleRoot,
    pub counts: SyncMerkleDebugCounts,
    pub newest_history_chunk_id: Option<String>,
    pub newest_history_max_played_at: Option<i64>,
}

/// Full sync-data response.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct SyncData {
    pub track_meta: Vec<SyncTrackMeta>,
    pub playlists: Vec<SyncPlaylist>,
    pub smart_playlists: Vec<SyncSmartPlaylist>,
    pub history: Vec<SyncHistoryEntry>,
}

#[derive(Debug)]
struct LocalTrackSyncState {
    is_liked: bool,
    play_count: i64,
    rarity: Option<String>,
    manually_edited: bool,
    date_added: Option<i64>,
}

impl SyncData {
    // This is the single place that defines what library data is exported to peers
    // and how the same domains are merged back into the local database.
    fn from_db(conn: &Connection) -> Self {
        Self {
            track_meta: load_sync_track_meta(conn),
            playlists: load_sync_playlists(conn),
            smart_playlists: load_sync_smart_playlists(conn),
            history: load_sync_history(conn),
        }
    }

    fn merge_into_library(&self, conn: &Connection) {
        let hash_to_id = build_local_hash_to_id_map(conn);
        merge_remote_track_metadata(conn, &hash_to_id, &self.track_meta);
        merge_remote_playlists(conn, &hash_to_id, &self.playlists);
        merge_remote_smart_playlists(conn, &self.smart_playlists);
        merge_remote_history(conn, &hash_to_id, &self.history);
    }
}

impl SyncTrackMeta {
    fn state(&self) -> SyncTrackState {
        SyncTrackState {
            is_liked: self.is_liked,
            play_count: self.play_count,
            rarity: self.rarity.clone(),
            manually_edited: self.manually_edited,
        }
    }
}

fn sync_track_meta_from_parts(hash: String, state: SyncTrackState, fields: SyncTrackFields) -> SyncTrackMeta {
    SyncTrackMeta {
        hash,
        is_liked: state.is_liked,
        play_count: state.play_count,
        rarity: state.rarity,
        manually_edited: state.manually_edited,
        fields,
    }
}

fn hash_json<T: Serialize>(value: &T) -> String {
    serde_json::to_vec(value)
        .map(|bytes| blake3::hash(&bytes).to_hex().to_string())
        .unwrap_or_default()
}

fn hash_history_events(events: &[SyncHistoryEntry]) -> String {
    let mut hasher = blake3::Hasher::new();
    for event in events {
        hasher.update(event.hash.as_bytes());
        hasher.update(&event.played_at.to_le_bytes());
    }
    hasher.finalize().to_hex().to_string()
}

fn hash_sync_track_state(state: &SyncTrackState) -> String {
    hash_json(state)
}

fn hash_sync_track_fields(fields: &SyncTrackFields) -> String {
    hash_json(fields)
}

fn dedup_track_meta_by_hash(track_meta: Vec<SyncTrackMeta>) -> Vec<SyncTrackMeta> {
    let mut by_hash: HashMap<String, SyncTrackMeta> = HashMap::new();
    for meta in track_meta {
        by_hash.entry(meta.hash.clone()).or_insert(meta);
    }
    let mut deduped: Vec<SyncTrackMeta> = by_hash.into_values().collect();
    deduped.sort_by(|a, b| a.hash.cmp(&b.hash));
    deduped
}

fn load_remote_tracks(conn: &Connection) -> Vec<RemoteTrack> {
    let mut tracks: Vec<RemoteTrack> = conn
        .prepare(
            "SELECT file_hash, path, title, artist, album, source_kind, source_path, cue_path \
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
                    source_kind: row.get(5)?,
                    source_path: row.get(6)?,
                    cue_path: row.get(7)?,
                    file_size: None,
                })
            })
            .map(|rows| rows.filter_map(|r| r.ok()).collect::<Vec<_>>())
            .unwrap_or_default()
        })
        .unwrap_or_default();
    tracks.sort_by(|a, b| a.hash.cmp(&b.hash).then_with(|| a.path.cmp(&b.path)));
    tracks
}

fn load_sync_track_state_by_hash(conn: &Connection, hash: &str) -> Option<SyncTrackState> {
    conn.query_row(
        "SELECT is_liked, play_count, rarity, manually_edited
         FROM tracks WHERE file_hash = ?1 LIMIT 1",
        params![hash],
        |row| {
            Ok(SyncTrackState {
                is_liked: row.get::<_, i64>(0).unwrap_or(0) != 0,
                play_count: row.get::<_, i64>(1).unwrap_or(0),
                rarity: row.get::<_, Option<String>>(2)?,
                manually_edited: row.get::<_, i64>(3).unwrap_or(0) != 0,
            })
        },
    )
    .ok()
}

fn load_sync_track_fields_by_hash(conn: &Connection, hash: &str) -> Option<SyncTrackFields> {
    conn.query_row(
        "SELECT title, artist, album, track_number, year, genre, tags, date_added
         FROM tracks WHERE file_hash = ?1 LIMIT 1",
        params![hash],
        |row| {
            Ok(SyncTrackFields {
                title: row.get(0)?,
                artist: row.get(1)?,
                album: row.get(2)?,
                track_number: row.get(3)?,
                year: row.get(4)?,
                genre: row.get(5)?,
                tags: row.get(6)?,
                date_added: row.get(7)?,
            })
        },
    )
    .ok()
}

fn load_sync_track_meta(conn: &Connection) -> Vec<SyncTrackMeta> {
    conn.prepare(
        "SELECT file_hash, is_liked, play_count, rarity, manually_edited,
                title, artist, album, track_number, year, genre, tags, date_added
         FROM tracks WHERE file_hash IS NOT NULL
         ORDER BY file_hash, id",
    )
    .map(|mut stmt| {
        stmt.query_map([], |row| {
            Ok(SyncTrackMeta {
                hash: row.get(0)?,
                is_liked: row.get::<_, i64>(1).unwrap_or(0) != 0,
                play_count: row.get::<_, i64>(2).unwrap_or(0),
                rarity: row.get(3)?,
                manually_edited: row.get::<_, i64>(4).unwrap_or(0) != 0,
                fields: SyncTrackFields {
                    title: row.get(5)?,
                    artist: row.get(6)?,
                    album: row.get(7)?,
                    track_number: row.get(8)?,
                    year: row.get(9)?,
                    genre: row.get(10)?,
                    tags: row.get(11)?,
                    date_added: row.get(12)?,
                },
            })
        })
        .map(|rows| rows.filter_map(|row| row.ok()).collect::<Vec<_>>())
        .unwrap_or_default()
    })
    .unwrap_or_default()
}

fn load_sync_playlists(conn: &Connection) -> Vec<SyncPlaylist> {
    conn.prepare("SELECT id, name FROM playlists ORDER BY created_at")
        .map(|mut stmt| {
            stmt.query_map([], |row| {
                let playlist_id: i64 = row.get(0)?;
                let name: String = row.get(1)?;
                Ok((playlist_id, name))
            })
            .map(|rows| rows.filter_map(|row| row.ok()).collect::<Vec<_>>())
            .unwrap_or_default()
        })
        .unwrap_or_default()
        .into_iter()
        .map(|(playlist_id, name)| {
            let track_hashes: Vec<String> = conn
                .prepare(
                    "SELECT t.file_hash FROM playlist_tracks pt
                     JOIN tracks t ON t.id = pt.track_id
                     WHERE pt.playlist_id = ?1 AND t.file_hash IS NOT NULL
                     ORDER BY pt.position, pt.id",
                )
                .and_then(|mut stmt| {
                    stmt.query_map(params![playlist_id], |row| row.get::<_, String>(0))
                        .map(|rows| rows.filter_map(|row| row.ok()).collect())
                })
                .unwrap_or_default();
            SyncPlaylist { name, track_hashes }
        })
        .collect()
}

fn load_sync_smart_playlists(conn: &Connection) -> Vec<SyncSmartPlaylist> {
    conn.prepare("SELECT id, name, match_mode, rules_json, updated_at FROM smart_playlists")
        .map(|mut stmt| {
            stmt.query_map([], |row| {
                Ok(SyncSmartPlaylist {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    match_mode: row.get(2)?,
                    rules_json: row.get(3)?,
                    updated_at: row.get(4)?,
                })
            })
            .map(|rows| rows.filter_map(|row| row.ok()).collect::<Vec<_>>())
            .unwrap_or_default()
        })
        .unwrap_or_default()
}

fn load_sync_history(conn: &Connection) -> Vec<SyncHistoryEntry> {
    conn.prepare(
        "SELECT t.file_hash, ph.played_at
         FROM play_history ph
         JOIN tracks t ON t.id = ph.track_id
         WHERE t.file_hash IS NOT NULL
         ORDER BY ph.played_at",
    )
    .map(|mut stmt| {
        stmt.query_map([], |row| {
            Ok(SyncHistoryEntry {
                hash: row.get(0)?,
                played_at: row.get(1)?,
            })
        })
        .map(|rows| rows.filter_map(|row| row.ok()).collect::<Vec<_>>())
        .unwrap_or_default()
    })
    .unwrap_or_default()
}

fn build_sync_content_hashes(remote_tracks: &[RemoteTrack]) -> Vec<SyncContentHashes> {
    let mut by_hash: BTreeMap<String, SyncContentHashes> = BTreeMap::new();
    for track in remote_tracks {
        by_hash.entry(track.hash.clone()).or_insert_with(|| {
            let descriptor_hash = hash_json(&(
                track.path.clone(),
                track.title.clone(),
                track.artist.clone(),
                track.album.clone(),
                track.source_kind.clone(),
                track.source_path.clone(),
                track.cue_path.clone(),
                track.file_size,
            ));
            SyncContentHashes {
                hash: track.hash.clone(),
                blob_hash: track.hash.clone(),
                descriptor_hash,
            }
        });
    }
    by_hash.into_values().collect()
}

fn build_sync_track_meta_hashes(track_meta: &[SyncTrackMeta]) -> Vec<SyncTrackMetaHashes> {
    let mut by_hash: BTreeMap<String, SyncTrackMetaHashes> = BTreeMap::new();
    for meta in track_meta {
        by_hash.entry(meta.hash.clone()).or_insert_with(|| {
            let state = meta.state();
            SyncTrackMetaHashes {
                hash: meta.hash.clone(),
                state_hash: hash_sync_track_state(&state),
                fields_hash: hash_sync_track_fields(&meta.fields),
            }
        });
    }
    by_hash.into_values().collect()
}

fn load_sync_track_meta_hashes(conn: &Connection) -> Vec<SyncTrackMetaHashes> {
    let track_meta = dedup_track_meta_by_hash(load_sync_track_meta(conn));
    build_sync_track_meta_hashes(&track_meta)
}

fn build_sync_playlist_hashes(playlists: &[SyncPlaylist]) -> Vec<SyncPlaylistHashes> {
    let mut hashes: Vec<SyncPlaylistHashes> = playlists
        .iter()
        .map(|playlist| SyncPlaylistHashes {
            name: playlist.name.clone(),
            identity_hash: hash_json(&playlist.name),
            tracks_hash: hash_json(&playlist.track_hashes),
        })
        .collect();
    hashes.sort_by(|a, b| a.name.cmp(&b.name));
    hashes
}

fn load_sync_playlist_hashes(conn: &Connection) -> Vec<SyncPlaylistHashes> {
    build_sync_playlist_hashes(&load_sync_playlists(conn))
}

fn build_sync_smart_playlist_hashes(playlists: &[SyncSmartPlaylist]) -> Vec<SyncSmartPlaylistHashes> {
    let mut hashes: Vec<SyncSmartPlaylistHashes> = playlists
        .iter()
        .map(|playlist| {
            let node_hash = hash_json(&(
                playlist.name.clone(),
                playlist.match_mode.clone(),
                playlist.rules_json.clone(),
                playlist.updated_at,
            ));
            SyncSmartPlaylistHashes {
                id: playlist.id.clone(),
                node_hash,
            }
        })
        .collect();
    hashes.sort_by(|a, b| a.id.cmp(&b.id));
    hashes
}

fn load_sync_smart_playlist_hashes(conn: &Connection) -> Vec<SyncSmartPlaylistHashes> {
    build_sync_smart_playlist_hashes(&load_sync_smart_playlists(conn))
}

fn history_chunk_bucket(played_at: i64) -> i64 {
    played_at.div_euclid(PLAY_HISTORY_CHUNK_SECONDS)
}

fn history_chunk_id(bucket: i64) -> String {
    format!("{bucket:012}")
}

fn build_sync_history_chunk(bucket: i64, mut events: Vec<SyncHistoryEntry>) -> Option<SyncHistoryChunk> {
    if events.is_empty() {
        return None;
    }
    events.sort_by(|a, b| a.played_at.cmp(&b.played_at).then_with(|| a.hash.cmp(&b.hash)));
    let min_played_at = events.first().map(|entry| entry.played_at).unwrap_or_default();
    let max_played_at = events.last().map(|entry| entry.played_at).unwrap_or_default();
    let event_count = events.len();
    let rolling_hash = hash_history_events(&events);
    Some(SyncHistoryChunk {
        chunk_id: history_chunk_id(bucket),
        range: SyncHistoryChunkRange {
            min_played_at,
            max_played_at,
        },
        event_count,
        rolling_hash,
        events,
    })
}

fn build_sync_history_chunks(history: &[SyncHistoryEntry]) -> Vec<SyncHistoryChunk> {
    let mut grouped: BTreeMap<i64, Vec<SyncHistoryEntry>> = BTreeMap::new();
    for entry in history {
        grouped
            .entry(history_chunk_bucket(entry.played_at))
            .or_default()
            .push(entry.clone());
    }

    grouped
        .into_iter()
        .filter_map(|(bucket, events)| build_sync_history_chunk(bucket, events))
        .collect()
}

fn load_sync_history_chunks(conn: &Connection) -> Vec<SyncHistoryChunk> {
    let history = load_sync_history(conn);
    build_sync_history_chunks(&history)
}

fn build_sync_history_chunk_summaries(chunks: &[SyncHistoryChunk]) -> Vec<SyncHistoryChunkSummary> {
    chunks
        .iter()
        .map(|chunk| SyncHistoryChunkSummary {
            chunk_id: chunk.chunk_id.clone(),
            range: chunk.range.clone(),
            event_count: chunk.event_count,
            rolling_hash: chunk.rolling_hash.clone(),
        })
        .collect()
}

fn load_sync_history_chunk_summaries(conn: &Connection) -> Vec<SyncHistoryChunkSummary> {
    build_sync_history_chunk_summaries(&load_sync_history_chunks(conn))
}

fn load_sync_history_chunk_by_id(conn: &Connection, chunk_id: &str) -> Option<SyncHistoryChunk> {
    let bucket = chunk_id.parse::<i64>().ok()?;
    let min_played_at = bucket.saturating_mul(PLAY_HISTORY_CHUNK_SECONDS);
    let max_played_at = min_played_at.saturating_add(PLAY_HISTORY_CHUNK_SECONDS - 1);
    let events: Vec<SyncHistoryEntry> = conn
        .prepare(
            "SELECT t.file_hash, ph.played_at
             FROM play_history ph
             JOIN tracks t ON t.id = ph.track_id
             WHERE t.file_hash IS NOT NULL
               AND ph.played_at >= ?1
               AND ph.played_at <= ?2
             ORDER BY ph.played_at, t.file_hash",
        )
        .and_then(|mut stmt| {
            stmt.query_map(params![min_played_at, max_played_at], |row| {
                Ok(SyncHistoryEntry {
                    hash: row.get(0)?,
                    played_at: row.get(1)?,
                })
            })
            .map(|rows| rows.filter_map(|row| row.ok()).collect())
        })
        .unwrap_or_default();

    build_sync_history_chunk(bucket, events)
}

fn build_sync_merkle_root(conn: &Connection) -> SyncMerkleRoot {
    let content_by_hash_hash = hash_json(&build_sync_content_hashes(&load_remote_tracks(conn)));
    let track_meta_by_hash_hash = hash_json(&load_sync_track_meta_hashes(conn));
    let playlists_by_name_hash = hash_json(&load_sync_playlist_hashes(conn));
    let smart_playlists_by_id_hash = hash_json(&load_sync_smart_playlist_hashes(conn));
    let play_history_log_hash = hash_json(&load_sync_history_chunk_summaries(conn));

    let library_state_hash = hash_json(&(
        track_meta_by_hash_hash.clone(),
        playlists_by_name_hash.clone(),
        smart_playlists_by_id_hash.clone(),
        play_history_log_hash.clone(),
    ));
    let root_hash = hash_json(&(content_by_hash_hash.clone(), library_state_hash.clone()));

    SyncMerkleRoot {
        version: SYNC_MERKLE_VERSION.to_string(),
        root_hash,
        content_by_hash_hash,
        library_state_hash,
        track_meta_by_hash_hash,
        playlists_by_name_hash,
        smart_playlists_by_id_hash,
        play_history_log_hash,
    }
}

fn build_sync_merkle_debug(conn: &Connection) -> SyncMerkleDebugResponse {
    let content_hashes = build_sync_content_hashes(&load_remote_tracks(conn));
    let track_meta_hashes = load_sync_track_meta_hashes(conn);
    let playlists = load_sync_playlists(conn);
    let smart_playlists = load_sync_smart_playlists(conn);
    let history_chunk_summaries = load_sync_history_chunk_summaries(conn);

    let newest_history_chunk = history_chunk_summaries.last();
    let history_events_total = history_chunk_summaries
        .iter()
        .map(|chunk| chunk.event_count)
        .sum::<usize>();

    SyncMerkleDebugResponse {
        app_version: APP_VERSION.to_string(),
        merkle_version: SYNC_MERKLE_VERSION.to_string(),
        root: build_sync_merkle_root(conn),
        counts: SyncMerkleDebugCounts {
            content_entries: content_hashes.len(),
            track_meta_entries: track_meta_hashes.len(),
            playlists: playlists.len(),
            smart_playlists: smart_playlists.len(),
            history_chunks: history_chunk_summaries.len(),
            history_events_total,
        },
        newest_history_chunk_id: newest_history_chunk.map(|chunk| chunk.chunk_id.clone()),
        newest_history_max_played_at: newest_history_chunk.map(|chunk| chunk.range.max_played_at),
    }
}

fn build_local_hash_to_id_map(conn: &Connection) -> HashMap<String, i64> {
    let mut hash_to_id = HashMap::new();
    if let Ok(mut stmt) = conn.prepare("SELECT file_hash, id FROM tracks WHERE file_hash IS NOT NULL") {
        if let Ok(rows) = stmt.query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))) {
            for row in rows.flatten() {
                hash_to_id.insert(row.0, row.1);
            }
        }
    }
    hash_to_id
}

fn load_local_track_sync_state(conn: &Connection, track_id: i64) -> Option<LocalTrackSyncState> {
    conn.query_row(
        "SELECT is_liked, play_count, rarity, manually_edited, date_added FROM tracks WHERE id = ?1",
        params![track_id],
        |row| {
            Ok(LocalTrackSyncState {
                is_liked: row.get::<_, i64>(0).unwrap_or(0) != 0,
                play_count: row.get::<_, i64>(1).unwrap_or(0),
                rarity: row.get::<_, Option<String>>(2)?,
                manually_edited: row.get::<_, i64>(3).unwrap_or(0) != 0,
                date_added: row.get::<_, Option<i64>>(4)?,
            })
        },
    )
    .ok()
}

fn apply_remote_manual_track_fields(conn: &Connection, track_id: i64, fields: &SyncTrackFields) {
    let _ = conn.execute(
        "UPDATE tracks
         SET title = ?1,
             artist = ?2,
             album = ?3,
             track_number = ?4,
             year = ?5,
             genre = ?6,
             tags = ?7,
             manually_edited = 1
         WHERE id = ?8",
        params![
            fields.title.clone(),
            fields.artist.clone(),
            fields.album.clone(),
            fields.track_number,
            fields.year,
            fields.genre.clone(),
            fields.tags.clone(),
            track_id,
        ],
    );
}

fn merge_remote_track_metadata(
    conn: &Connection,
    hash_to_id: &HashMap<String, i64>,
    remote_tracks: &[SyncTrackMeta],
) {
    // is_liked: OR (liked anywhere → liked everywhere)
    // play_count: MAX (take whichever is higher)
    // rarity: keep remote if local is NULL
    // date_added: keep earliest seen timestamp
    // manually_edited fields: adopt remote title/artist/album/track_number/year/genre/tags only when local is not edited
    for remote_track in remote_tracks {
        let Some(&track_id) = hash_to_id.get(&remote_track.hash) else { continue };
        let Some(local) = load_local_track_sync_state(conn, track_id) else { continue };

        let merged_liked = local.is_liked || remote_track.is_liked;
        let merged_play_count = local.play_count.max(remote_track.play_count);
        let merged_rarity = local.rarity.or_else(|| remote_track.rarity.clone());
        let merged_date_added = match (local.date_added, remote_track.fields.date_added) {
            (Some(local_date), Some(remote_date)) => Some(local_date.min(remote_date)),
            (Some(local_date), None) => Some(local_date),
            (None, remote_date) => remote_date,
        };

        let _ = conn.execute(
            "UPDATE tracks
             SET is_liked = ?1,
                 play_count = ?2,
                 rarity = ?3,
                 date_added = COALESCE(?5, date_added)
             WHERE id = ?4",
            params![merged_liked as i64, merged_play_count, merged_rarity, track_id, merged_date_added],
        );

        if remote_track.manually_edited && !local.manually_edited {
            apply_remote_manual_track_fields(conn, track_id, &remote_track.fields);
        }
    }
}

fn merge_remote_playlists(
    conn: &Connection,
    hash_to_id: &HashMap<String, i64>,
    remote_playlists: &[SyncPlaylist],
) {
    // By name: if playlist exists locally, union track hashes; if not, create it.
    for remote_playlist in remote_playlists {
        let existing_id: Option<i64> = conn
            .query_row(
                "SELECT id FROM playlists WHERE name = ?1",
                params![remote_playlist.name],
                |row| row.get(0),
            )
            .ok();
        let playlist_id = match existing_id {
            Some(id) => id,
            None => {
                let _ = conn.execute("INSERT INTO playlists (name) VALUES (?1)", params![remote_playlist.name]);
                conn.last_insert_rowid()
            }
        };
        let max_position: i64 = conn
            .query_row(
                "SELECT COALESCE(MAX(position), -1) FROM playlist_tracks WHERE playlist_id = ?1",
                params![playlist_id],
                |row| row.get(0),
            )
            .unwrap_or(-1);
        let mut next_position = max_position + 1;
        for hash in &remote_playlist.track_hashes {
            if let Some(&track_id) = hash_to_id.get(hash) {
                let result = conn.execute(
                    "INSERT OR IGNORE INTO playlist_tracks (playlist_id, track_id, position) VALUES (?1, ?2, ?3)",
                    params![playlist_id, track_id, next_position],
                );
                if result.map(|changed| changed > 0).unwrap_or(false) {
                    next_position += 1;
                }
            }
        }
    }
}

fn merge_remote_smart_playlists(conn: &Connection, remote_playlists: &[SyncSmartPlaylist]) {
    // By UUID: if same id exists, take whichever has higher updated_at. If id doesn't exist, insert.
    for remote_playlist in remote_playlists {
        let local_updated: Option<i64> = conn
            .query_row(
                "SELECT updated_at FROM smart_playlists WHERE id = ?1",
                params![remote_playlist.id],
                |row| row.get(0),
            )
            .ok();
        match local_updated {
            Some(local_updated_at) if local_updated_at >= remote_playlist.updated_at => {}
            _ => {
                let _ = conn.execute(
                    "INSERT INTO smart_playlists (id, name, match_mode, rules_json, updated_at)
                     VALUES (?1, ?2, ?3, ?4, ?5)
                     ON CONFLICT(id) DO UPDATE SET
                       name = excluded.name,
                       match_mode = excluded.match_mode,
                       rules_json = excluded.rules_json,
                       updated_at = excluded.updated_at",
                    params![
                        remote_playlist.id,
                        remote_playlist.name,
                        remote_playlist.match_mode,
                        remote_playlist.rules_json,
                        remote_playlist.updated_at,
                    ],
                );
            }
        }
    }
}

fn merge_remote_history(
    conn: &Connection,
    hash_to_id: &HashMap<String, i64>,
    remote_history: &[SyncHistoryEntry],
) {
    // Union: insert (track_id, played_at) pairs that don't exist yet.
    for remote_entry in remote_history {
        let Some(&track_id) = hash_to_id.get(&remote_entry.hash) else { continue };
        let exists: bool = conn
            .query_row(
                "SELECT 1 FROM play_history WHERE track_id = ?1 AND played_at = ?2",
                params![track_id, remote_entry.played_at],
                |_| Ok(()),
            )
            .is_ok();
        if !exists {
            let _ = conn.execute(
                "INSERT INTO play_history (track_id, played_at) VALUES (?1, ?2)",
                params![track_id, remote_entry.played_at],
            );
        }
    }
}

// ── State ─────────────────────────────────────────────────────────────────────

pub struct SyncState {
    inner: Arc<Mutex<SyncInner>>,
}

struct SyncInner {
    enabled: bool,
    server_started: bool,
    in_flight_hashes: HashSet<String>,
    in_flight_sources: HashSet<String>,
}

impl SyncState {
    pub fn new(enabled: bool) -> Self {
        SyncState {
            inner: Arc::new(Mutex::new(SyncInner {
                enabled,
                server_started: false,
                in_flight_hashes: HashSet::new(),
                in_flight_sources: HashSet::new(),
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
    let bind_addresses = [format!("0.0.0.0:{SYNC_PORT}"), format!("[::]:{SYNC_PORT}")];
    let mut listeners_started = 0usize;

    for bind_addr in bind_addresses {
        match tiny_http::Server::http(&bind_addr) {
            Ok(server) => {
                listeners_started += 1;
                let conn = Arc::clone(&conn);
                let data_dir = data_dir.clone();
                let app = app.clone();
                thread::spawn(move || {
                    eprintln!("[sync] HTTP server listening on {bind_addr}");
                    for request in server.incoming_requests() {
                        let conn = Arc::clone(&conn);
                        let dir = data_dir.clone();
                        let app = app.clone();
                        thread::spawn(move || handle_request(request, conn, dir, app));
                    }
                });
            }
            Err(e) => {
                eprintln!("[sync] failed to bind {bind_addr}: {e}");
            }
        }
    }

    if listeners_started == 0 {
        eprintln!("[sync] cannot bind port {SYNC_PORT} on IPv4 or IPv6");
    }
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
        serve_tracks(request, &conn, &data_dir);
    } else if url == "/sync-data" {
        serve_sync_data(request, &conn);
    } else if url == "/sync-merkle/root" {
        serve_sync_merkle_root(request, &conn);
    } else if url == "/sync-merkle/debug" {
        serve_sync_merkle_debug(request, &conn);
    } else if url == "/sync-merkle/track-meta/index" {
        serve_sync_merkle_track_meta_index(request, &conn);
    } else if let Some(hash) = url.strip_prefix("/sync-merkle/track-meta/state/") {
        serve_sync_merkle_track_meta_state(request, &conn, hash);
    } else if let Some(hash) = url.strip_prefix("/sync-merkle/track-meta/fields/") {
        serve_sync_merkle_track_meta_fields(request, &conn, hash);
    } else if url == "/sync-merkle/playlists" {
        serve_sync_merkle_playlists(request, &conn);
    } else if url == "/sync-merkle/smart-playlists" {
        serve_sync_merkle_smart_playlists(request, &conn);
    } else if url == "/sync-merkle/history/chunks" {
        serve_sync_merkle_history_chunks(request, &conn);
    } else if let Some(chunk_id) = url.strip_prefix("/sync-merkle/history/chunk/") {
        serve_sync_merkle_history_chunk(request, &conn, chunk_id);
    } else if let Some(hash) = url.strip_prefix("/cue/") {
        let hash = hash.to_string();
        serve_cue_file(request, &hash, &conn, &data_dir);
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

fn attach_remote_track_file_sizes(data_dir: &Path, tracks: &mut [RemoteTrack]) {
    for track in tracks {
        let source_rel = remote_track_source_rel(track);
        let abs = rel_path_to_abs(data_dir, source_rel);
        track.file_size = std::fs::metadata(abs).ok().map(|meta| meta.len());
    }
}

fn serve_tracks(request: tiny_http::Request, conn: &Arc<Mutex<Connection>>, data_dir: &Path) {
    let body = {
        let c = conn.lock().unwrap();
        let mut tracks = load_remote_tracks(&c);
        attach_remote_track_file_sizes(data_dir, &mut tracks);
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
            "SELECT COALESCE(NULLIF(source_path, ''), path) FROM tracks WHERE file_hash = ?1 LIMIT 1",
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

fn serve_cue_file(request: tiny_http::Request, hash: &str, conn: &Arc<Mutex<Connection>>, data_dir: &Path) {
    let rel: Option<String> = conn
        .lock()
        .unwrap()
        .query_row(
            "SELECT cue_path FROM tracks WHERE file_hash = ?1 AND cue_path IS NOT NULL LIMIT 1",
            params![hash],
            |row| row.get(0),
        )
        .ok();
    let Some(rel) = rel else {
        let _ = request.respond(tiny_http::Response::empty(404));
        return;
    };
    let abs = rel_path_to_abs(data_dir, &rel);
    match std::fs::read(&abs) {
        Ok(data) => {
            let content_type = Header::from_bytes(b"Content-Type", b"application/octet-stream").unwrap();
            let _ = request.respond(tiny_http::Response::from_data(data).with_header(content_type));
        }
        Err(_) => {
            let _ = request.respond(tiny_http::Response::empty(404));
        }
    }
}

fn serve_sync_data(request: tiny_http::Request, conn: &Arc<Mutex<Connection>>) {
    let body = {
        let conn = conn.lock().unwrap();
        serde_json::to_vec(&SyncData::from_db(&conn)).unwrap_or_default()
    };
    let ct = Header::from_bytes(b"Content-Type", b"application/json").unwrap();
    let _ = request.respond(tiny_http::Response::from_data(body).with_header(ct));
}

fn serve_sync_merkle_root(request: tiny_http::Request, conn: &Arc<Mutex<Connection>>) {
    let payload = {
        let conn = conn.lock().unwrap();
        build_sync_merkle_root(&conn)
    };
    respond_json(request, &payload);
}

fn serve_sync_merkle_debug(request: tiny_http::Request, conn: &Arc<Mutex<Connection>>) {
    let payload = {
        let conn = conn.lock().unwrap();
        build_sync_merkle_debug(&conn)
    };
    respond_json(request, &payload);
}

fn serve_sync_merkle_track_meta_index(request: tiny_http::Request, conn: &Arc<Mutex<Connection>>) {
    let payload = {
        let conn = conn.lock().unwrap();
        load_sync_track_meta_hashes(&conn)
    };
    respond_json(request, &payload);
}

fn serve_sync_merkle_track_meta_state(
    request: tiny_http::Request,
    conn: &Arc<Mutex<Connection>>,
    hash: &str,
) {
    if hash.trim().is_empty() {
        respond_status(request, 404);
        return;
    }
    let payload = {
        let conn = conn.lock().unwrap();
        load_sync_track_state_by_hash(&conn, hash).map(|state| SyncTrackStatePayload {
            hash: hash.to_string(),
            state,
        })
    };
    match payload {
        Some(payload) => respond_json(request, &payload),
        None => respond_status(request, 404),
    }
}

fn serve_sync_merkle_track_meta_fields(
    request: tiny_http::Request,
    conn: &Arc<Mutex<Connection>>,
    hash: &str,
) {
    if hash.trim().is_empty() {
        respond_status(request, 404);
        return;
    }
    let payload = {
        let conn = conn.lock().unwrap();
        load_sync_track_fields_by_hash(&conn, hash).map(|fields| SyncTrackFieldsPayload {
            hash: hash.to_string(),
            fields,
        })
    };
    match payload {
        Some(payload) => respond_json(request, &payload),
        None => respond_status(request, 404),
    }
}

fn serve_sync_merkle_playlists(request: tiny_http::Request, conn: &Arc<Mutex<Connection>>) {
    let payload = {
        let conn = conn.lock().unwrap();
        load_sync_playlists(&conn)
    };
    respond_json(request, &payload);
}

fn serve_sync_merkle_smart_playlists(request: tiny_http::Request, conn: &Arc<Mutex<Connection>>) {
    let payload = {
        let conn = conn.lock().unwrap();
        load_sync_smart_playlists(&conn)
    };
    respond_json(request, &payload);
}

fn serve_sync_merkle_history_chunks(request: tiny_http::Request, conn: &Arc<Mutex<Connection>>) {
    let payload = {
        let conn = conn.lock().unwrap();
        load_sync_history_chunk_summaries(&conn)
    };
    respond_json(request, &payload);
}

fn serve_sync_merkle_history_chunk(
    request: tiny_http::Request,
    conn: &Arc<Mutex<Connection>>,
    chunk_id: &str,
) {
    if chunk_id.trim().is_empty() {
        respond_status(request, 404);
        return;
    }
    let payload = {
        let conn = conn.lock().unwrap();
        load_sync_history_chunk_by_id(&conn, chunk_id)
    };
    match payload {
        Some(payload) => respond_json(request, &payload),
        None => respond_status(request, 404),
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

fn peer_get_json<T: DeserializeOwned>(
    client: &reqwest::blocking::Client,
    base_urls: &[String],
    path: &str,
) -> Result<T, String> {
    let bytes = peer_get(client, base_urls, path)?;
    serde_json::from_slice::<T>(&bytes).map_err(|e| e.to_string())
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
            item_done: None,
            item_total: None,
        },
    );
}

fn emit_download_item(
    app: &AppHandle,
    peer: &str,
    device_name: Option<&str>,
    device_emoji: Option<&str>,
    total: usize,
    done: usize,
    label: &str,
    item_done: u64,
    item_total: Option<u64>,
) {
    let _ = app.emit(
        "sync-progress",
        SyncProgress {
            peer: peer.to_string(),
            device_name: device_name.map(|s| s.to_string()),
            device_emoji: device_emoji.map(|s| s.to_string()),
            phase: "download".to_string(),
            total,
            done,
            message: Some(label.to_string()),
            item_done: Some(item_done),
            item_total,
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

fn claim_in_flight_source(app: &AppHandle, source_rel: &str) -> bool {
    let sync = app.state::<SyncState>();
    let mut inner = sync.inner.lock().unwrap();
    if inner.in_flight_sources.contains(source_rel) {
        return false;
    }

    inner.in_flight_sources.insert(source_rel.to_string());
    true
}

fn release_in_flight_source(app: &AppHandle, source_rel: &str) {
    let sync = app.state::<SyncState>();
    let mut inner = sync.inner.lock().unwrap();
    inner.in_flight_sources.remove(source_rel);
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

fn file_hash_hex(path: &Path) -> Option<String> {
    let bytes = std::fs::read(path).ok()?;
    Some(blake3::hash(&bytes).to_hex().to_string())
}

fn path_with_hash_suffix(path: &Path, short_hash: &str, attempt: usize) -> PathBuf {
    let parent = path.parent().unwrap_or_else(|| Path::new(""));
    let stem = path
        .file_stem()
        .map(|value| value.to_string_lossy().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "track".to_string());
    let ext = path
        .extension()
        .map(|value| value.to_string_lossy().to_string())
        .filter(|value| !value.is_empty());

    let suffix = if attempt == 0 {
        format!(" [{short_hash}]")
    } else {
        format!(" [{short_hash}-{attempt}]")
    };
    let file_name = match ext {
        Some(ext) => format!("{stem}{suffix}.{ext}"),
        None => format!("{stem}{suffix}"),
    };
    parent.join(file_name)
}

fn choose_download_save_path(initial_path: &Path, remote_hash: &str) -> PathBuf {
    if !initial_path.exists() {
        return initial_path.to_path_buf();
    }
    if file_hash_hex(initial_path).as_deref() == Some(remote_hash) {
        return initial_path.to_path_buf();
    }

    let short_hash = &remote_hash[..remote_hash.len().min(8)];
    for attempt in 0..10_000usize {
        let candidate = path_with_hash_suffix(initial_path, short_hash, attempt);
        if !candidate.exists() {
            return candidate;
        }
        if file_hash_hex(&candidate).as_deref() == Some(remote_hash) {
            return candidate;
        }
    }

    let fallback_attempt = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos() as usize;
    path_with_hash_suffix(initial_path, short_hash, fallback_attempt)
}

fn remote_track_source_rel(track: &RemoteTrack) -> &str {
    track
        .source_path
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or(track.path.as_str())
}

fn remote_track_cue_rel(track: &RemoteTrack) -> Option<&str> {
    track.cue_path.as_deref().filter(|value| !value.trim().is_empty())
}

fn is_cue_remote_track(track: &RemoteTrack) -> bool {
    track.source_kind.as_deref() == Some("cue") || remote_track_cue_rel(track).is_some()
}

fn rel_filename(rel: &str) -> &str {
    rel.rsplit(|ch| ch == '/' || ch == '\\')
        .next()
        .filter(|value| !value.is_empty())
        .unwrap_or(rel)
}

fn download_cue_file_for_track(
    client: &reqwest::blocking::Client,
    base_urls: &[String],
    track: &RemoteTrack,
    save_dir: &Path,
    downloaded_cues: &mut HashSet<String>,
) {
    let Some(cue_rel) = remote_track_cue_rel(track) else {
        return;
    };
    if downloaded_cues.contains(cue_rel) {
        return;
    }
    let cue_filename = rel_filename(cue_rel);
    let cue_save_path = save_dir.join(cue_filename);
    if cue_save_path.exists() {
        downloaded_cues.insert(cue_rel.to_string());
        return;
    }
    if let Some(parent) = cue_save_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let path = format!("/cue/{}", track.hash);
    match peer_get(client, base_urls, &path) {
        Ok(bytes) => {
            if let Err(error) = std::fs::write(&cue_save_path, &bytes) {
                eprintln!(
                    "[sync] failed to write cue file for hash {} to {}: {error}",
                    track.hash,
                    cue_save_path.display()
                );
            } else {
                downloaded_cues.insert(cue_rel.to_string());
            }
        }
        Err(error) => {
            eprintln!("[sync] failed to download cue file for hash {}: {error}", track.hash);
        }
    }
}

struct DownloadProgressContext<'a> {
    app: &'a AppHandle,
    peer_name: &'a str,
    device_name: Option<&'a str>,
    device_emoji: Option<&'a str>,
    total: usize,
    done: usize,
    label: &'a str,
    item_total_hint: Option<u64>,
}

fn peer_download_with_progress(
    client: &reqwest::blocking::Client,
    base_urls: &[String],
    path: &str,
    progress: &DownloadProgressContext<'_>,
) -> Result<Vec<u8>, String> {
    let mut last_err = String::from("no reachable peer address");
    for base in base_urls {
        let url = format!("{}{}", base, path);
        match client.get(&url).send() {
            Ok(mut resp) if resp.status().is_success() => {
                let item_total = resp.content_length().or(progress.item_total_hint);
                let mut bytes = Vec::with_capacity(item_total.unwrap_or(0).min(usize::MAX as u64) as usize);
                let mut buf = [0u8; 64 * 1024];
                let mut item_done = 0u64;
                let mut last_emit = Instant::now() - Duration::from_millis(250);

                emit_download_item(
                    progress.app,
                    progress.peer_name,
                    progress.device_name,
                    progress.device_emoji,
                    progress.total,
                    progress.done,
                    progress.label,
                    0,
                    item_total,
                );

                loop {
                    let read = resp.read(&mut buf).map_err(|e| e.to_string())?;
                    if read == 0 {
                        break;
                    }
                    bytes.extend_from_slice(&buf[..read]);
                    item_done = item_done.saturating_add(read as u64);
                    if last_emit.elapsed() >= Duration::from_millis(250) {
                        emit_download_item(
                            progress.app,
                            progress.peer_name,
                            progress.device_name,
                            progress.device_emoji,
                            progress.total,
                            progress.done,
                            progress.label,
                            item_done,
                            item_total,
                        );
                        last_emit = Instant::now();
                    }
                }

                emit_download_item(
                    progress.app,
                    progress.peer_name,
                    progress.device_name,
                    progress.device_emoji,
                    progress.total,
                    progress.done,
                    progress.label,
                    item_done,
                    item_total.or(Some(item_done)),
                );
                return Ok(bytes);
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

fn merge_remote_track_metadata_via_merkle(
    conn: &Arc<Mutex<Connection>>,
    client: &reqwest::blocking::Client,
    base_urls: &[String],
) -> Result<(), String> {
    let remote_hashes = peer_get_json::<Vec<SyncTrackMetaHashes>>(client, base_urls, "/sync-merkle/track-meta/index")?;

    let (local_hashes_by_hash, local_track_meta_by_hash): (
        HashMap<String, SyncTrackMetaHashes>,
        HashMap<String, SyncTrackMeta>,
    ) = {
        let c = conn.lock().unwrap();
        let local_track_meta = dedup_track_meta_by_hash(load_sync_track_meta(&c));
        let local_hashes = build_sync_track_meta_hashes(&local_track_meta);
        (
            local_hashes
                .into_iter()
                .map(|entry| (entry.hash.clone(), entry))
                .collect(),
            local_track_meta
                .into_iter()
                .map(|entry| (entry.hash.clone(), entry))
                .collect(),
        )
    };

    let mut changed_remote_track_meta: Vec<SyncTrackMeta> = Vec::new();
    for remote in remote_hashes {
        let local_hashes = local_hashes_by_hash.get(&remote.hash);
        let needs_state = local_hashes
            .map(|local| local.state_hash != remote.state_hash)
            .unwrap_or(true);
        let needs_fields = local_hashes
            .map(|local| local.fields_hash != remote.fields_hash)
            .unwrap_or(true);
        if !needs_state && !needs_fields {
            continue;
        }

        let state = if needs_state {
            let path = format!("/sync-merkle/track-meta/state/{}", remote.hash);
            peer_get_json::<SyncTrackStatePayload>(client, base_urls, &path)?.state
        } else if let Some(local_meta) = local_track_meta_by_hash.get(&remote.hash) {
            local_meta.state()
        } else {
            let path = format!("/sync-merkle/track-meta/state/{}", remote.hash);
            peer_get_json::<SyncTrackStatePayload>(client, base_urls, &path)?.state
        };

        let fields = if needs_fields {
            let path = format!("/sync-merkle/track-meta/fields/{}", remote.hash);
            peer_get_json::<SyncTrackFieldsPayload>(client, base_urls, &path)?.fields
        } else if let Some(local_meta) = local_track_meta_by_hash.get(&remote.hash) {
            local_meta.fields.clone()
        } else {
            let path = format!("/sync-merkle/track-meta/fields/{}", remote.hash);
            peer_get_json::<SyncTrackFieldsPayload>(client, base_urls, &path)?.fields
        };

        changed_remote_track_meta.push(sync_track_meta_from_parts(remote.hash, state, fields));
    }

    if changed_remote_track_meta.is_empty() {
        return Ok(());
    }

    let c = conn.lock().unwrap();
    let hash_to_id = build_local_hash_to_id_map(&c);
    merge_remote_track_metadata(&c, &hash_to_id, &changed_remote_track_meta);
    Ok(())
}

fn merge_remote_playlists_via_merkle(
    conn: &Arc<Mutex<Connection>>,
    client: &reqwest::blocking::Client,
    base_urls: &[String],
) -> Result<(), String> {
    let remote_playlists = peer_get_json::<Vec<SyncPlaylist>>(client, base_urls, "/sync-merkle/playlists")?;
    let c = conn.lock().unwrap();
    let hash_to_id = build_local_hash_to_id_map(&c);
    merge_remote_playlists(&c, &hash_to_id, &remote_playlists);
    Ok(())
}

fn merge_remote_smart_playlists_via_merkle(
    conn: &Arc<Mutex<Connection>>,
    client: &reqwest::blocking::Client,
    base_urls: &[String],
) -> Result<(), String> {
    let remote_playlists =
        peer_get_json::<Vec<SyncSmartPlaylist>>(client, base_urls, "/sync-merkle/smart-playlists")?;
    let c = conn.lock().unwrap();
    merge_remote_smart_playlists(&c, &remote_playlists);
    Ok(())
}

fn merge_remote_history_via_merkle(
    conn: &Arc<Mutex<Connection>>,
    client: &reqwest::blocking::Client,
    base_urls: &[String],
) -> Result<(), String> {
    let local_chunk_hashes_by_id: HashMap<String, String> = {
        let c = conn.lock().unwrap();
        load_sync_history_chunk_summaries(&c)
            .into_iter()
            .map(|summary| (summary.chunk_id, summary.rolling_hash))
            .collect()
    };

    let remote_chunk_summaries =
        peer_get_json::<Vec<SyncHistoryChunkSummary>>(client, base_urls, "/sync-merkle/history/chunks")?;

    let mut changed_events: Vec<SyncHistoryEntry> = Vec::new();
    for summary in remote_chunk_summaries {
        let is_same_chunk = local_chunk_hashes_by_id
            .get(&summary.chunk_id)
            .map(|local_hash| local_hash == &summary.rolling_hash)
            .unwrap_or(false);
        if is_same_chunk {
            continue;
        }

        let path = format!("/sync-merkle/history/chunk/{}", summary.chunk_id);
        let chunk = peer_get_json::<SyncHistoryChunk>(client, base_urls, &path)?;
        changed_events.extend(chunk.events);
    }

    if changed_events.is_empty() {
        return Ok(());
    }

    let c = conn.lock().unwrap();
    let hash_to_id = build_local_hash_to_id_map(&c);
    merge_remote_history(&c, &hash_to_id, &changed_events);
    Ok(())
}

fn is_http_404_error(error: &str) -> bool {
    error.contains("HTTP 404")
}

fn merge_sync_data_via_merkle(
    conn: &Arc<Mutex<Connection>>,
    client: &reqwest::blocking::Client,
    base_urls: &[String],
) -> Result<bool, String> {
    let remote_root = match peer_get_json::<SyncMerkleRoot>(client, base_urls, "/sync-merkle/root") {
        Ok(root) => root,
        Err(error) if is_http_404_error(&error) => {
            // Peer is running older sync protocol and does not expose Merkle endpoints.
            return Ok(false);
        }
        Err(error) => return Err(error),
    };

    let local_root = {
        let c = conn.lock().unwrap();
        build_sync_merkle_root(&c)
    };

    if local_root.library_state_hash == remote_root.library_state_hash {
        return Ok(true);
    }

    if local_root.track_meta_by_hash_hash != remote_root.track_meta_by_hash_hash {
        merge_remote_track_metadata_via_merkle(conn, client, base_urls)?;
    }
    if local_root.playlists_by_name_hash != remote_root.playlists_by_name_hash {
        merge_remote_playlists_via_merkle(conn, client, base_urls)?;
    }
    if local_root.smart_playlists_by_id_hash != remote_root.smart_playlists_by_id_hash {
        merge_remote_smart_playlists_via_merkle(conn, client, base_urls)?;
    }
    if local_root.play_history_log_hash != remote_root.play_history_log_hash {
        merge_remote_history_via_merkle(conn, client, base_urls)?;
    }

    Ok(true)
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
    if total > 0 {
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
    }

    // 3 ── Download missing files ──────────────────────────────────────────────
    // Organized by metadata: [Artist]/[Album]/filename (or [Artist]/filename, or just filename)

    let mut done = 0usize;
    let mut added = 0usize;
    let mut downloaded_sources: HashSet<String> = HashSet::new();
    let mut downloaded_cues: HashSet<String> = HashSet::new();
    for track in &missing {
        // Build target path based on metadata
        // Get artist and album, sanitizing them for use as directory names
        let artist = track.artist.as_deref().map(|a| {
            a.chars()
                .map(|ch| match ch {
                    '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
                    c if c.is_control() => '_',
                    c => c,
                })
                .collect::<String>()
                .trim()
                .trim_matches('.')
                .to_string()
        }).filter(|a| !a.is_empty());
        
        let album = track.album.as_deref().map(|a| {
            a.chars()
                .map(|ch| match ch {
                    '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
                    c if c.is_control() => '_',
                    c => c,
                })
                .collect::<String>()
                .trim()
                .trim_matches('.')
                .to_string()
        }).filter(|a| !a.is_empty());

        // Build the directory path
        let mut save_dir = data_dir.clone();
        if let Some(artist_name) = &artist {
            save_dir.push(artist_name);
            if let Some(album_name) = &album {
                save_dir.push(album_name);
            }
        }

        // Get the filename from the physical source path when this is a virtual track.
        let source_rel = remote_track_source_rel(track);
        let filename = rel_filename(source_rel);
        let requested_save_path = save_dir.join(filename);
        let save_path = if is_cue_remote_track(track) {
            requested_save_path.clone()
        } else {
            choose_download_save_path(&requested_save_path, &track.hash)
        };
        if save_path != requested_save_path {
            eprintln!(
                "[sync] path collision for hash {}: requested {}, using {}",
                track.hash,
                requested_save_path.display(),
                save_path.display()
            );
        }

        if downloaded_sources.contains(source_rel) {
            download_cue_file_for_track(&client, &base_urls, track, &save_dir, &mut downloaded_cues);
            done += 1;
            let label = track.title.as_deref()
                .filter(|t| !t.is_empty())
                .unwrap_or(filename);
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

        // Recheck the DB at download time and reserve this hash across
        // concurrent sync workers so two peers can't fetch the same track.
        if !claim_in_flight_hash(&app, &conn, &track.hash) {
            download_cue_file_for_track(&client, &base_urls, track, &save_dir, &mut downloaded_cues);
            done += 1;
            let label = track.title.as_deref()
                .filter(|t| !t.is_empty())
                .unwrap_or_else(|| filename);
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

        // Fast path: if we already have the exact same-content file, only register hash mapping.
        if save_path.exists() {
            if let Err(e) = register_downloaded_hash(&conn, &data_dir, &save_path, &track.hash) {
                eprintln!("[sync] failed to register existing file for hash {}: {e}", track.hash);
            } else {
                added += 1;
                downloaded_sources.insert(source_rel.to_string());
                download_cue_file_for_track(&client, &base_urls, track, &save_dir, &mut downloaded_cues);
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

        let source_claimed = if is_cue_remote_track(track) {
            if !claim_in_flight_source(&app, source_rel) {
                release_in_flight_hash(&app, &track.hash);
                done += 1;
                let label = track.title.as_deref()
                    .filter(|t| !t.is_empty())
                    .unwrap_or(filename);
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
            true
        } else {
            false
        };

        let label = track.title.as_deref()
            .filter(|t| !t.is_empty())
            .unwrap_or(filename);
        let path = format!("/file/{}", track.hash);
        let progress = DownloadProgressContext {
            app: &app,
            peer_name: &peer_name,
            device_name: remote_device_name.as_deref(),
            device_emoji: remote_device_emoji.as_deref(),
            total,
            done,
            label,
            item_total_hint: track.file_size,
        };
        if let Ok(bytes) = peer_download_with_progress(&client, &base_urls, &path, &progress) {
            if std::fs::write(&save_path, &bytes).is_ok() {
                if let Err(e) = register_downloaded_hash(&conn, &data_dir, &save_path, &track.hash) {
                    eprintln!("[sync] failed to register downloaded file for hash {}: {e}", track.hash);
                } else {
                    added += 1;
                    downloaded_sources.insert(source_rel.to_string());
                    download_cue_file_for_track(&client, &base_urls, track, &save_dir, &mut downloaded_cues);
                }
            }
        }
        if source_claimed {
            release_in_flight_source(&app, source_rel);
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
    if total > 0 {
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
    }

    // 5 ── Sync metadata, playlists, smart playlists, history ─────────────────
    emit(
        &app,
        &peer_name,
        remote_device_name.as_deref(),
        remote_device_emoji.as_deref(),
        "merging",
        total,
        done,
        Some("Merging library data...".to_string()),
    );
    let merkle_merged = match merge_sync_data_via_merkle(&conn, &client, &base_urls) {
        Ok(used_merkle) => used_merkle,
        Err(error) => {
            eprintln!("[sync] merkle merge failed: {error}");
            false
        }
    };
    if !merkle_merged {
        if let Ok(sd_bytes) = peer_get(&client, &base_urls, "/sync-data") {
            if let Ok(remote) = serde_json::from_slice::<SyncData>(&sd_bytes) {
                merge_sync_data(&app, remote);
            }
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
        Some(if added == 0 {
            "Library metadata synced".to_string()
        } else {
            format!("{added} new track(s) added")
        }),
    );
}

// ── Merge helpers ─────────────────────────────────────────────────────────────

fn merge_sync_data(app: &AppHandle, remote: SyncData) {
    let conn = app.state::<crate::library::LibraryState>().conn();
    let c = conn.lock().unwrap();
    remote.merge_into_library(&c);
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
