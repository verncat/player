//! Versioned PM protocol for explicit, temporary track sharing between Player friends.
use crate::{library::LibraryState, profile::LocalProfileState, soulseek::SoulseekState};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::{
    fs::File,
    io::{Read, Seek},
    time::Duration,
};
use tauri::Manager;

const PREFIX: &str = "adhdplayer:track:v1:";
const MAX_SIZE: u64 = 2 * 1024 * 1024 * 1024;
#[derive(Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
enum Packet {
    Request {
        id: String,
        from: String,
        to: String,
        title: String,
        artist: String,
        album: String,
    },
    Ready {
        id: String,
        from: String,
        to: String,
        path: String,
        size: u64,
        hash: String,
    },
    Decline {
        id: String,
        from: String,
        to: String,
    },
}
impl Packet {
    fn identifiers(&self) -> (&str, &str, &str) {
        match self {
            Self::Request { id, from, to, .. }
            | Self::Ready { id, from, to, .. }
            | Self::Decline { id, from, to } => (id, from, to),
        }
    }
    fn body(&self) -> Result<String, String> {
        serde_json::to_string(self)
            .map(|json| format!("{PREFIX}{json}"))
            .map_err(|e| e.to_string())
    }
}
fn parse(body: &str) -> Option<Packet> {
    if body.len() > 4096 {
        return None;
    }
    let packet: Packet = serde_json::from_str(body.strip_prefix(PREFIX)?).ok()?;
    let (id, from, to) = packet.identifiers();
    for value in [id, from, to] {
        if uuid::Uuid::parse_str(value).ok()?.to_string() != value {
            return None;
        }
    }
    if from == to {
        return None;
    }
    match &packet {
        Packet::Request {
            title,
            artist,
            album,
            ..
        } if title.trim().is_empty()
            || [title, artist, album]
                .iter()
                .any(|s| s.len() > 512 || s.contains('\0')) =>
        {
            return None
        }
        Packet::Ready {
            id,
            path,
            size,
            hash,
            ..
        } if *size == 0
            || *size > MAX_SIZE
            || !valid_alias(id, path)
            || hash.len() != 64
            || !hash
                .bytes()
                .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()) =>
        {
            return None
        }
        _ => {}
    }
    Some(packet)
}
fn valid_alias(id: &str, path: &str) -> bool {
    let Some(extension) = path.strip_prefix(&format!("Player requests\\{id}\\track.")) else {
        return false;
    };
    matches!(
        extension,
        "mp3" | "flac" | "ogg" | "opus" | "aac" | "m4a" | "wav" | "wv" | "ape"
    )
}

#[derive(Serialize)]
pub struct TrackRequest {
    pub id: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub state: String,
    pub incoming: bool,
    pub error: String,
}
pub(crate) fn init_schema(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch("CREATE TABLE IF NOT EXISTS friend_track_requests (
        owner TEXT NOT NULL, account TEXT NOT NULL, username TEXT NOT NULL, id TEXT NOT NULL,
        friend_id TEXT NOT NULL, incoming INTEGER NOT NULL, title TEXT NOT NULL, artist TEXT NOT NULL, album TEXT NOT NULL,
        state TEXT NOT NULL, created INTEGER NOT NULL DEFAULT (unixepoch()),
        path TEXT, size INTEGER, hash TEXT, error TEXT NOT NULL DEFAULT '',
        PRIMARY KEY(owner, account, username, id));
        UPDATE friend_track_requests SET state = 'failed', error = 'Transfer interrupted. Request the track again.' WHERE state IN ('approving', 'downloading');")
}

pub(crate) fn receive(
    conn: &Connection,
    owner: &str,
    account: &str,
    message: &soulseek_rs::actor::server_actor::UserMessage,
) -> rusqlite::Result<()> {
    let Some(packet) = parse(&message.message) else {
        return Ok(());
    };
    let (id, from, to) = packet.identifiers();
    if to != owner {
        return Ok(());
    }
    // Discovery hints cannot authenticate an identity. Require an existing friend and mapped PM account.
    if crate::private_messages::authorize_target(conn, owner, from, &message.username).is_err() {
        return Ok(());
    }
    match &packet {
        Packet::Request {
            title,
            artist,
            album,
            ..
        } => {
            let pending: i64 = conn.query_row("SELECT count(*) FROM friend_track_requests WHERE owner = ?1 AND state = 'requested' AND created > unixepoch() - 600", [owner], |row| row.get(0))?;
            if pending < 32 {
                conn.execute("INSERT OR IGNORE INTO friend_track_requests(owner, account, username, id, friend_id, incoming, title, artist, album, state, created)
                    VALUES (?1,?2,?3,?4,?5,1,?6,?7,?8,'requested',?9)", params![owner, account, message.username, id, from, title, artist, album, message.timestamp])?;
            }
        }
        Packet::Ready {
            path, size, hash, ..
        } => {
            conn.execute("UPDATE friend_track_requests SET state = 'ready', path = ?6, size = ?7, hash = ?8
                WHERE owner = ?1 AND account = ?2 AND username = ?3 AND id = ?4 AND friend_id = ?5 AND incoming = 0 AND state = 'requested' AND created > unixepoch() - 600",
                params![owner, account, message.username, id, from, path, *size as i64, hash])?;
        }
        Packet::Decline { .. } => {
            conn.execute("UPDATE friend_track_requests SET state = 'declined' WHERE owner = ?1 AND account = ?2 AND username = ?3 AND id = ?4 AND friend_id = ?5 AND incoming = 0 AND state = 'requested'",
                params![owner, account, message.username, id, from])?;
        }
    }
    Ok(())
}
pub(crate) fn message_card(
    conn: &Connection,
    owner: &str,
    account: &str,
    username: &str,
    body: &str,
) -> rusqlite::Result<Option<TrackRequest>> {
    let Some(Packet::Request { id, .. }) = parse(body) else {
        return Ok(None);
    };
    conn.query_row("SELECT id,title,artist,album,state,incoming,error,created FROM friend_track_requests WHERE owner=?1 AND account=?2 AND username=?3 AND id=?4",
        params![owner,account,username,id], |row| {
            let state: String = row.get(4)?;
            let created: i64 = row.get(7)?;
            let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs() as i64;
            Ok(TrackRequest { id: row.get(0)?, title: row.get(1)?, artist: row.get(2)?, album: row.get(3)?,
                state: if matches!(state.as_str(), "requested" | "ready") && now - created >= 600 { "expired".into() } else { state }, incoming: row.get(5)?, error: row.get(6)? })
        }).optional()
}
pub(crate) fn display_body(body: &str) -> String {
    match parse(body) {
        Some(Packet::Request { title, artist, .. }) => format!("Track request: {title}\n{artist}"),
        Some(Packet::Ready { .. }) => "Track approved for download.".into(),
        Some(Packet::Decline { .. }) => "Track request declined.".into(),
        None => body.into(),
    }
}

#[tauri::command]
pub async fn request_friend_track(
    profile_id: String,
    account: String,
    username: String,
    library: tauri::State<'_, LibraryState>,
    profile: tauri::State<'_, LocalProfileState>,
    soulseek: tauri::State<'_, SoulseekState>,
    discord: tauri::State<'_, crate::discord_presence::DiscordPresenceState>,
) -> Result<(), String> {
    let track = discord
        .social
        .friend(&profile_id)
        .and_then(|friend| friend.track)
        .ok_or("This friend is not sharing a current track through Discord.")?;
    let owner = profile.info().user_id;
    let id = uuid::Uuid::new_v4().to_string();
    let packet = Packet::Request {
        id: id.clone(),
        from: owner.clone(),
        to: profile_id.clone(),
        title: track.title.clone(),
        artist: track.artist.clone(),
        album: track.album.clone(),
    };
    let body = packet.body()?;
    if parse(&body).is_none() {
        return Err("This activity has unsupported track metadata.".into());
    }
    {
        let connection = library.conn();
        let conn = connection.lock().map_err(|e| e.to_string())?;
        crate::private_messages::authorize_target(&conn, &owner, &profile_id, &username)?;
        let recent: i64 = conn.query_row("SELECT count(*) FROM friend_track_requests WHERE owner=?1 AND account=?2 AND username=?3 AND incoming=0 AND state='requested' AND created > unixepoch()-600", params![owner,account,username], |row| row.get(0)).map_err(|e| e.to_string())?;
        if recent > 0 {
            return Err("A track request is already waiting for a response.".into());
        }
        conn.execute("INSERT INTO friend_track_requests(owner,account,username,id,friend_id,incoming,title,artist,album,state) VALUES (?1,?2,?3,?4,?5,0,?6,?7,?8,'requested')",
            params![owner,account,username,id,profile_id,track.title,track.artist,track.album]).map_err(|e| e.to_string())?;
    }
    let result = crate::private_messages::send_friend_message(
        profile_id,
        account.clone(),
        username.clone(),
        body,
        library.clone(),
        profile,
        soulseek,
    )
    .await;
    if result.is_err() {
        set_state(
            &library,
            &owner,
            &account,
            &username,
            &id,
            "failed",
            "Could not confirm sending. Check the chat before requesting again.",
        )?;
    }
    result
}

fn set_state(
    library: &LibraryState,
    owner: &str,
    account: &str,
    username: &str,
    id: &str,
    state: &str,
    error: &str,
) -> Result<(), String> {
    library.conn().lock().map_err(|e| e.to_string())?.execute("UPDATE friend_track_requests SET state=?5,error=?6 WHERE owner=?1 AND account=?2 AND username=?3 AND id=?4",
        params![owner,account,username,id,state,error]).map_err(|e| e.to_string())?;
    Ok(())
}

fn open_requested_file(
    library: &LibraryState,
    track: &TrackRequest,
) -> Result<(File, String), String> {
    let paths = {
        let connection = library.conn();
        let conn = connection.lock().map_err(|e| e.to_string())?;
        let mut query = conn.prepare("SELECT path FROM tracks WHERE COALESCE(title,'Unknown track')=?1 AND COALESCE(artist,'Unknown artist')=?2 AND COALESCE(album,'')=?3 AND source_kind='file' LIMIT 2").map_err(|e| e.to_string())?;
        let rows = query
            .query_map(params![track.title, track.artist, track.album], |row| {
                row.get::<_, String>(0)
            })
            .map_err(|e| e.to_string())?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|e| e.to_string())?
    };
    if paths.len() != 1 {
        return Err("A unique local audio file could not be found for this track. CUE segments cannot be shared yet.".into());
    }
    let root = library
        .data_dir()
        .canonicalize()
        .map_err(|e| e.to_string())?;
    let path = root
        .join(&paths[0])
        .canonicalize()
        .map_err(|e| e.to_string())?;
    if !path.starts_with(&root) {
        return Err("The audio file is outside your library.".into());
    }
    let extension = path
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_lowercase();
    if !valid_alias(
        &track.id,
        &format!("Player requests\\{}\\track.{extension}", track.id),
    ) {
        return Err("Unsupported audio file type.".into());
    }
    let file = File::open(path).map_err(|e| e.to_string())?;
    let size = file.metadata().map_err(|e| e.to_string())?.len();
    if size == 0 || size > MAX_SIZE {
        return Err("The audio file must be between 1 byte and 2 GB.".into());
    }
    Ok((file, extension))
}
fn hash_file(file: &mut File) -> Result<String, String> {
    let mut hasher = blake3::Hasher::new();
    let mut buffer = [0_u8; 65536];
    file.rewind().map_err(|e| e.to_string())?;
    loop {
        let read = file.read(&mut buffer).map_err(|e| e.to_string())?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    file.rewind().map_err(|e| e.to_string())?;
    Ok(hasher.finalize().to_hex().to_string())
}

#[tauri::command]
pub async fn respond_friend_track(
    profile_id: String,
    account: String,
    username: String,
    request_id: String,
    accept: bool,
    library: tauri::State<'_, LibraryState>,
    profile: tauri::State<'_, LocalProfileState>,
    soulseek: tauri::State<'_, SoulseekState>,
) -> Result<(), String> {
    let owner = profile.info().user_id;
    let card = {
        let connection = library.conn();
        let conn = connection.lock().map_err(|e| e.to_string())?;
        crate::private_messages::authorize_target(&conn, &owner, &profile_id, &username)?;
        let body = Packet::Request {
            id: request_id.clone(),
            from: profile_id.clone(),
            to: owner.clone(),
            title: "lookup".into(),
            artist: String::new(),
            album: String::new(),
        }
        .body()?;
        let card = message_card(&conn, &owner, &account, &username, &body)
            .map_err(|e| e.to_string())?
            .ok_or("Track request not found.")?;
        if !card.incoming || card.state != "requested" {
            return Err("This request has already been handled or has expired.".into());
        }
        let count = conn.execute("UPDATE friend_track_requests SET state='approving' WHERE owner=?1 AND account=?2 AND username=?3 AND id=?4 AND friend_id=?5 AND incoming=1 AND state='requested'", params![owner,account,username,request_id,profile_id]).map_err(|e| e.to_string())?;
        if count != 1 {
            return Err("This request has already been handled.".into());
        }
        card
    };
    let result = async {
        let client = soulseek.discovery_client(&library, &profile).await?;
        let mut alias = None;
        let packet = if accept {
            let (mut file, extension) = open_requested_file(&library, &card)?;
            let (file, hash) = tauri::async_runtime::spawn_blocking(move || {
                hash_file(&mut file).map(|hash| (file, hash))
            })
            .await
            .map_err(|e| e.to_string())??;
            let path = format!("Player requests\\{}\\track.{extension}", card.id);
            let size = client
                .upload_shares()
                .add(username.clone(), path.clone(), file)
                .map_err(|e| e.to_string())?;
            alias = Some(path.clone());
            Packet::Ready {
                id: card.id.clone(),
                from: owner.clone(),
                to: profile_id.clone(),
                path,
                size,
                hash,
            }
        } else {
            Packet::Decline {
                id: card.id.clone(),
                from: owner.clone(),
                to: profile_id.clone(),
            }
        };
        let result = crate::private_messages::send_friend_message(
            profile_id,
            account.clone(),
            username.clone(),
            packet.body()?,
            library.clone(),
            profile,
            soulseek,
        )
        .await;
        if let Err(error) = result {
            if let Some(alias) = alias {
                client.upload_shares().remove(&alias);
            }
            return Err(error);
        }
        Ok(())
    }
    .await;
    set_state(
        &library,
        &owner,
        &account,
        &username,
        &card.id,
        if result.is_ok() {
            if accept {
                "shared"
            } else {
                "declined"
            }
        } else {
            "failed"
        },
        result.as_ref().err().map(String::as_str).unwrap_or(""),
    )?;
    result
}

#[derive(Clone)]
struct Ready {
    owner: String,
    account: String,
    username: String,
    id: String,
    friend_id: String,
    path: String,
    size: u64,
    hash: String,
}
fn claim_ready(library: &LibraryState, owner: &str, account: &str) -> Result<Vec<Ready>, String> {
    let connection = library.conn();
    let conn = connection.lock().map_err(|e| e.to_string())?;
    let active: i64 = conn
        .query_row(
            "SELECT count(*) FROM friend_track_requests WHERE state='downloading'",
            [],
            |row| row.get(0),
        )
        .map_err(|e| e.to_string())?;
    let slots = (4 - active).max(0);
    let mut query = conn.prepare("SELECT username,id,friend_id,path,size,hash FROM friend_track_requests WHERE owner=?1 AND account=?2 AND state='ready' AND incoming=0 AND created > unixepoch()-600 LIMIT ?3").map_err(|e| e.to_string())?;
    let rows = query
        .query_map(params![owner, account, slots], |row| {
            Ok(Ready {
                owner: owner.into(),
                account: account.into(),
                username: row.get(0)?,
                id: row.get(1)?,
                friend_id: row.get(2)?,
                path: row.get(3)?,
                size: row.get::<_, i64>(4)? as u64,
                hash: row.get(5)?,
            })
        })
        .map_err(|e| e.to_string())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| e.to_string())?;
    for row in &rows {
        conn.execute("UPDATE friend_track_requests SET state='downloading' WHERE owner=?1 AND account=?2 AND username=?3 AND id=?4 AND state='ready'", params![owner,account,row.username,row.id]).map_err(|e| e.to_string())?;
    }
    Ok(rows)
}

async fn download(app: &tauri::AppHandle, request: &Ready) -> Result<(), String> {
    let library = app.state::<LibraryState>();
    let profile = app.state::<LocalProfileState>();
    let soulseek = app.state::<SoulseekState>();
    let client = soulseek.discovery_client(&library, &profile).await?;
    let settings = library.get_device_settings().map_err(|e| e.to_string())?;
    if profile.info().user_id != request.owner
        || !settings.soulseek_enabled
        || settings.soulseek_username.trim() != request.account
    {
        return Err("Profile or Soulseek account changed.".into());
    }
    {
        let connection = library.conn();
        let conn = connection.lock().map_err(|e| e.to_string())?;
        crate::private_messages::authorize_target(
            &conn,
            &request.owner,
            &request.friend_id,
            &request.username,
        )?;
    }
    let root = library.data_dir().join(".soulseek-temp");
    std::fs::create_dir_all(&root).map_err(|e| e.to_string())?;
    let staging = tempfile::tempdir_in(root).map_err(|e| e.to_string())?;
    let (_, mut handle) = client
        .download(
            request.path.as_str(),
            request.username.clone(),
            request.size,
            staging.path().to_string_lossy().into(),
            Some(Duration::from_secs(60)),
            Some(Duration::from_secs(120)),
        )
        .map_err(|e| e.to_string())?;
    let timeout = tokio::time::sleep(Duration::from_secs(600));
    tokio::pin!(timeout);
    loop {
        tokio::select! {
            _ = &mut timeout => return Err("Download timed out.".into()),
            status = handle.recv() => match status {
                Some(soulseek_rs::DownloadStatus::Completed) => break,
                Some(soulseek_rs::DownloadStatus::Failed | soulseek_rs::DownloadStatus::TimedOut | soulseek_rs::DownloadStatus::Cancelled) | None => return Err("The Soulseek transfer failed. Both players need a reachable Soulseek connection.".into()),
                _ => {}
            }
        }
        let latest = library.get_device_settings().map_err(|e| e.to_string())?;
        if profile.info().user_id != request.owner
            || !latest.soulseek_enabled
            || latest.soulseek_username.trim() != request.account
        {
            return Err("Profile or Soulseek account changed.".into());
        }
    }
    let name = request
        .path
        .rsplit('\\')
        .next()
        .ok_or("Invalid file name")?;
    let path = staging.path().join(name);
    let expected = request.hash.clone();
    let size = request.size;
    let checked = path.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let mut file = File::open(&checked).map_err(|e| e.to_string())?;
        if file.metadata().map_err(|e| e.to_string())?.len() != size
            || hash_file(&mut file)? != expected
        {
            return Err("Downloaded audio failed its integrity check.".to_string());
        }
        Ok(())
    })
    .await
    .map_err(|e| e.to_string())??;
    let folder = library.data_dir().join("Friend tracks").join(&request.id);
    std::fs::create_dir_all(&folder).map_err(|e| e.to_string())?;
    if !folder
        .canonicalize()
        .map_err(|e| e.to_string())?
        .starts_with(
            library
                .data_dir()
                .canonicalize()
                .map_err(|e| e.to_string())?,
        )
    {
        return Err("Download destination is outside your library.".into());
    }
    let settings = library.get_device_settings().map_err(|e| e.to_string())?;
    if profile.info().user_id != request.owner
        || !settings.soulseek_enabled
        || settings.soulseek_username.trim() != request.account
    {
        return Err("Profile or Soulseek account changed.".into());
    }
    let destination = folder.join(name);
    // A unique request directory avoids overwriting an existing library track.
    let mut output = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&destination)
        .map_err(|e| e.to_string())?;
    let mut input = File::open(path).map_err(|e| e.to_string())?;
    std::io::copy(&mut input, &mut output).map_err(|e| e.to_string())?;
    let connection = library.conn();
    let conn = connection.lock().map_err(|e| e.to_string())?;
    crate::library::index_file(&conn, library.data_dir(), &destination)
        .map_err(|e| e.to_string())?;
    use tauri::Emitter;
    let _ = app.emit("library-changed", ());
    Ok(())
}

pub fn start(app: tauri::AppHandle) {
    tauri::async_runtime::spawn(async move {
        loop {
            let library = app.state::<LibraryState>();
            let profile = app.state::<LocalProfileState>();
            if let Ok(settings) = library.get_device_settings() {
                if settings.soulseek_enabled {
                    if let Ok(requests) = claim_ready(
                        &library,
                        &profile.info().user_id,
                        settings.soulseek_username.trim(),
                    ) {
                        for request in requests {
                            let app = app.clone();
                            tauri::async_runtime::spawn(async move {
                                let result = download(&app, &request).await;
                                let _ = set_state(
                                    &app.state::<LibraryState>(),
                                    &request.owner,
                                    &request.account,
                                    &request.username,
                                    &request.id,
                                    if result.is_ok() {
                                        "completed"
                                    } else {
                                        "failed"
                                    },
                                    result.as_ref().err().map(String::as_str).unwrap_or(""),
                                );
                            });
                        }
                    }
                }
            }
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn protocol_rejects_unknown_versions_paths_and_oversized_files() {
        let id = uuid::Uuid::new_v4().to_string();
        let from = uuid::Uuid::new_v4().to_string();
        let to = uuid::Uuid::new_v4().to_string();
        let valid = Packet::Ready {
            id: id.clone(),
            from,
            to,
            path: format!("Player requests\\{id}\\track.opus"),
            size: 42,
            hash: "a".repeat(64),
        }
        .body()
        .unwrap();
        assert!(parse(&valid).is_some());
        assert!(parse(&valid.replace("v1:", "v2:")).is_none());
        assert!(parse(&valid.replace("track.opus", "..\\private.key")).is_none());
        assert!(parse(&valid.replace("\"size\":42", "\"size\":99999999999")).is_none());
    }
    #[test]
    fn ready_packets_require_a_matching_outgoing_request_and_friend_account() {
        let conn = Connection::open_in_memory().unwrap();
        crate::friends::init_schema(&conn).unwrap();
        let owner = uuid::Uuid::new_v4().to_string();
        let friend = uuid::Uuid::new_v4().to_string();
        let id = uuid::Uuid::new_v4().to_string();
        conn.execute(
            "INSERT INTO friends(owner_id,profile_id,soulseek_users) VALUES (?1,?2,'[\"alice\"]')",
            params![owner, friend],
        )
        .unwrap();
        let body = Packet::Request {
            id: id.clone(),
            from: owner.clone(),
            to: friend.clone(),
            title: "Track".into(),
            artist: "Artist".into(),
            album: String::new(),
        }
        .body()
        .unwrap();
        conn.execute("INSERT INTO friend_track_requests(owner,account,username,id,friend_id,incoming,title,artist,album,state) VALUES (?1,'me','alice',?2,?3,0,'Track','Artist','','requested')", params![owner,id,friend]).unwrap();
        let ready = Packet::Ready {
            id: id.clone(),
            from: friend.clone(),
            to: owner.clone(),
            path: format!("Player requests\\{id}\\track.flac"),
            size: 100,
            hash: "a".repeat(64),
        }
        .body()
        .unwrap();
        let message = |username: &str| {
            soulseek_rs::actor::server_actor::UserMessage::new(
                1,
                1,
                username.into(),
                ready.clone(),
                true,
            )
        };
        receive(&conn, &owner, "me", &message("eve")).unwrap();
        assert_eq!(
            message_card(&conn, &owner, "me", "alice", &body)
                .unwrap()
                .unwrap()
                .state,
            "requested"
        );
        receive(&conn, &owner, "other-account", &message("alice")).unwrap();
        assert_eq!(
            message_card(&conn, &owner, "me", "alice", &body)
                .unwrap()
                .unwrap()
                .state,
            "requested"
        );
        receive(&conn, &owner, "me", &message("alice")).unwrap();
        assert_eq!(
            message_card(&conn, &owner, "me", "alice", &body)
                .unwrap()
                .unwrap()
                .state,
            "ready"
        );
        receive(&conn, &owner, "me", &message("alice")).unwrap();
        assert_eq!(
            message_card(&conn, &owner, "me", "alice", &body)
                .unwrap()
                .unwrap()
                .state,
            "ready"
        );
        conn.execute(
            "UPDATE friend_track_requests SET created=unixepoch()-601",
            [],
        )
        .unwrap();
        assert_eq!(
            message_card(&conn, &owner, "me", "alice", &body)
                .unwrap()
                .unwrap()
                .state,
            "expired"
        );
        conn.execute(
            "UPDATE friend_track_requests SET state='requested',created=unixepoch()",
            [],
        )
        .unwrap();
        let decline = Packet::Decline {
            id: id.clone(),
            from: friend,
            to: owner.clone(),
        }
        .body()
        .unwrap();
        receive(
            &conn,
            &owner,
            "me",
            &soulseek_rs::actor::server_actor::UserMessage::new(
                2,
                2,
                "alice".into(),
                decline,
                true,
            ),
        )
        .unwrap();
        receive(&conn, &owner, "me", &message("alice")).unwrap();
        assert_eq!(
            message_card(&conn, &owner, "me", "alice", &body)
                .unwrap()
                .unwrap()
                .state,
            "declined"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn approved_file_transfers_between_two_clients_over_soulseek_peer_connections() {
        use soulseek_rs::{message::Message, Client, ClientSettings};
        use tokio::{
            io::{AsyncReadExt, AsyncWriteExt},
            net::TcpListener,
        };
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let server_port = listener.local_addr().unwrap().port();
        let addresses = std::sync::Arc::new(std::sync::Mutex::new(std::collections::HashMap::<
            String,
            u32,
        >::new()));
        let shared = addresses.clone();
        let server = tokio::spawn(async move {
            let mut tasks = Vec::new();
            for _ in 0..2 {
                let (mut socket, _) = listener.accept().await.unwrap();
                let shared = shared.clone();
                tasks.push(tokio::spawn(async move {
                    let mut username = String::new();
                    loop {
                        let Ok(size) = socket.read_u32_le().await else {
                            break;
                        };
                        assert!(size < 8192);
                        let mut bytes = vec![0; size as usize];
                        if socket.read_exact(&mut bytes).await.is_err() {
                            break;
                        }
                        let mut packet = Message::new_with_data(bytes);
                        match packet.read_int32() {
                            1 => {
                                username = packet.read_string();
                                socket
                                    .write_all(
                                        &Message::new()
                                            .write_int32(1)
                                            .write_int8(1)
                                            .write_string("welcome")
                                            .write_int32(0)
                                            .write_string("")
                                            .write_bool(false)
                                            .get_buffer(),
                                    )
                                    .await
                                    .unwrap();
                            }
                            2 => {
                                shared
                                    .lock()
                                    .unwrap()
                                    .insert(username.clone(), packet.read_int32());
                            }
                            3 => {
                                let target = packet.read_string();
                                let port =
                                    shared.lock().unwrap().get(&target).copied().unwrap_or(0);
                                socket
                                    .write_all(
                                        &Message::new()
                                            .write_int32(3)
                                            .write_string(&target)
                                            .write_int32(u32::from_be_bytes([127, 0, 0, 1]))
                                            .write_int32(port)
                                            .write_int32(0)
                                            .write_int32(0)
                                            .get_buffer(),
                                    )
                                    .await
                                    .unwrap();
                            }
                            _ => {}
                        }
                    }
                }));
            }
            for task in tasks {
                task.await.unwrap();
            }
        });
        async fn client(name: &str, server_port: u16) -> Client {
            let port = TcpListener::bind("127.0.0.1:0")
                .await
                .unwrap()
                .local_addr()
                .unwrap()
                .port();
            let mut settings = ClientSettings::new(name, "synthetic-password");
            settings.server_address =
                soulseek_rs::PeerAddress::new("127.0.0.1".into(), server_port);
            settings.listen_port = u32::from(port);
            settings.shared_files = 0;
            settings.shared_folders = 0;
            settings.reconnect_settings = soulseek_rs::client::ReconnectSettings::Disabled;
            let client = Client::with_settings(settings);
            client.connect().await.unwrap();
            client
        }
        let alice = client("alice", server_port).await;
        let bob = client("bob", server_port).await;
        tokio::time::timeout(Duration::from_secs(5), async {
            while addresses.lock().unwrap().len() != 2 {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .unwrap();
        let source = tempfile::tempdir().unwrap();
        let destination = tempfile::tempdir().unwrap();
        let audio = vec![0x5a; 400_000];
        std::fs::write(source.path().join("audio.flac"), &audio).unwrap();
        let alias = "Player requests\\test\\track.flac";
        // QueueUpload alone must never publish a local file.
        let (_, mut denied) = bob
            .download(
                alias,
                "alice".into(),
                audio.len() as u64,
                destination.path().to_string_lossy().into(),
                Some(Duration::from_secs(5)),
                Some(Duration::from_secs(10)),
            )
            .unwrap();
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                match denied.recv().await {
                    Some(soulseek_rs::DownloadStatus::Failed) => break,
                    Some(soulseek_rs::DownloadStatus::Completed) => {
                        panic!("Unapproved file was shared")
                    }
                    None => panic!("Missing rejection"),
                    _ => {}
                }
            }
        })
        .await
        .unwrap();
        assert!(!destination.path().join("track.flac").exists());
        drop(denied);
        alice
            .upload_shares()
            .add(
                "bob".into(),
                alias.into(),
                File::open(source.path().join("audio.flac")).unwrap(),
            )
            .unwrap();
        let (_, mut handle) = bob
            .download(
                alias,
                "alice".into(),
                audio.len() as u64,
                destination.path().to_string_lossy().into(),
                Some(Duration::from_secs(5)),
                Some(Duration::from_secs(10)),
            )
            .unwrap();
        tokio::time::timeout(Duration::from_secs(15), async {
            loop {
                match handle.recv().await {
                    Some(soulseek_rs::DownloadStatus::Completed) => break,
                    Some(
                        soulseek_rs::DownloadStatus::Failed
                        | soulseek_rs::DownloadStatus::TimedOut
                        | soulseek_rs::DownloadStatus::Cancelled,
                    )
                    | None => panic!("Approved transfer failed"),
                    _ => {}
                }
            }
        })
        .await
        .expect("Approved transfer did not finish");
        assert_eq!(
            std::fs::read(destination.path().join("track.flac")).unwrap(),
            audio
        );
        alice.disconnect();
        bob.disconnect();
        tokio::time::timeout(Duration::from_secs(5), server)
            .await
            .unwrap()
            .unwrap();
    }
}
