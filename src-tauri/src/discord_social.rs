//! Official Social SDK behind an optional native adapter. No Discord private APIs.
use crate::discord_presence::PlaybackPresence;
#[cfg(any(discord_social_sdk, test))]
use base64::Engine;
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};

#[derive(Clone, Default, Deserialize, Serialize)]
pub struct ListeningTrack {
    pub title: String,
    pub artist: String,
    pub album: String,
}
#[derive(Clone, Serialize)]
pub struct DiscordFriend {
    pub profile_id: String,
    pub id: String,
    pub name: String,
    pub avatar: String,
    pub track: Option<ListeningTrack>,
}
#[derive(Clone, Default, Serialize)]
pub struct SocialStatus {
    supported: bool,
    pub connecting: bool,
    pub connected: bool,
    pub error: String,
    pub friends: Vec<DiscordFriend>,
}
#[derive(Default, Deserialize)]
#[cfg(any(discord_social_sdk, test))]
struct Snapshot {
    connected: bool,
    error: String,
    users: Vec<SocialUser>,
}
#[derive(Deserialize)]
#[cfg(any(discord_social_sdk, test))]
struct SocialUser {
    id: String,
    name: String,
    avatar: String,
    url: String,
    title: String,
    artist: String,
    album: String,
}

#[cfg(any(discord_social_sdk, test))]
pub(crate) fn profile_from_url(url: &str) -> Option<String> {
    let key = url.strip_prefix("adhd://v1/")?;
    let bytes = base64::engine::general_purpose::STANDARD.decode(key).ok()?;
    if bytes.len() != 32 {
        return None;
    }
    Some(uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_OID, &bytes).to_string())
}

#[cfg(any(discord_social_sdk, test))]
fn matched(snapshot: Snapshot) -> SocialStatus {
    let mut friends = Vec::new();
    if snapshot.connected {
        for user in snapshot.users {
            let Some(profile_id) = profile_from_url(&user.url) else {
                continue;
            };
            let track = if user.title.trim().is_empty() {
                None
            } else {
                Some(ListeningTrack {
                    title: user.title,
                    artist: user.artist,
                    album: user.album,
                })
            };
            friends.push(DiscordFriend {
                profile_id,
                id: user.id,
                name: user.name,
                avatar: user.avatar,
                track,
            });
        }
    }
    // Ambiguous advertisements never select an arbitrary Discord account.
    let counts = friends
        .iter()
        .fold(std::collections::HashMap::new(), |mut counts, friend| {
            *counts.entry(friend.profile_id.clone()).or_insert(0) += 1;
            counts
        });
    friends.retain(|friend| counts[&friend.profile_id] == 1);
    SocialStatus {
        supported: cfg!(discord_social_sdk),
        connected: snapshot.connected,
        connecting: false,
        error: snapshot.error,
        friends,
    }
}

#[derive(Default)]
pub struct SocialState {
    pub(crate) active: std::sync::atomic::AtomicBool,
    status: Arc<Mutex<SocialStatus>>,
    #[cfg(discord_social_sdk)]
    sender: Mutex<Option<std::sync::mpsc::Sender<Command>>>,
}
impl SocialState {
    pub fn status(&self) -> SocialStatus {
        let mut status = self
            .status
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        status.supported = cfg!(discord_social_sdk);
        status
    }
    pub fn friend(&self, profile_id: &str) -> Option<DiscordFriend> {
        self.status()
            .friends
            .into_iter()
            .find(|friend| friend.profile_id == profile_id)
    }
    pub fn update(&self, enabled: bool, playback: Option<PlaybackPresence>) {
        #[cfg(discord_social_sdk)]
        if let Some(sender) = self
            .sender
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
        {
            let _ = sender.send(Command::Presence(enabled, playback));
        }
        #[cfg(not(discord_social_sdk))]
        let _ = (enabled, playback);
    }
    pub fn connect(&self, id: u64) -> Result<(), String> {
        #[cfg(discord_social_sdk)]
        {
            let mut sender = self
                .sender
                .lock()
                .map_err(|_| "Discord adapter unavailable")?;
            if sender.is_none() {
                let (tx, rx) = std::sync::mpsc::channel();
                let status = self.status.clone();
                std::thread::Builder::new()
                    .name("discord-social".into())
                    .spawn(move || native::run(id, rx, status))
                    .map_err(|e| e.to_string())?;
                *sender = Some(tx);
            }
            self.active.store(true, std::sync::atomic::Ordering::SeqCst);
            sender
                .as_ref()
                .unwrap()
                .send(Command::Connect)
                .map_err(|_| "Discord adapter stopped".to_string())?;
            *self
                .status
                .lock()
                .map_err(|_| "Discord adapter unavailable")? = SocialStatus {
                supported: true,
                connecting: true,
                ..Default::default()
            };
            return Ok(());
        }
        #[cfg(not(discord_social_sdk))]
        {
            let _ = id;
            Err("This build does not include the Discord Social SDK.".into())
        }
    }
    pub fn disconnect(&self) {
        #[cfg(discord_social_sdk)]
        if let Some(sender) = self.sender.lock().unwrap_or_else(|e| e.into_inner()).take() {
            let (tx, rx) = std::sync::mpsc::channel();
            let _ = sender.send(Command::Stop(tx));
            let _ = rx.recv_timeout(std::time::Duration::from_secs(3));
        }
        self.active
            .store(false, std::sync::atomic::Ordering::SeqCst);
        *self.status.lock().unwrap_or_else(|e| e.into_inner()) = SocialStatus::default();
    }
}

#[cfg(discord_social_sdk)]
enum Command {
    Connect,
    Presence(bool, Option<PlaybackPresence>),
    Stop(std::sync::mpsc::Sender<()>),
}

#[cfg(discord_social_sdk)]
mod native {
    use super::*;
    use std::{
        ffi::{c_char, c_void, CString},
        time::{Duration, Instant},
    };
    unsafe extern "C" {
        fn player_social_create(app_id: u64) -> *mut c_void;
        fn player_social_destroy(ptr: *mut c_void);
        fn player_social_poll(ptr: *mut c_void, buffer: *mut c_char, capacity: usize) -> usize;
        fn player_social_presence(
            ptr: *mut c_void,
            title: *const c_char,
            artist: *const c_char,
            album: *const c_char,
            url: *const c_char,
            playing: bool,
            start: u64,
            end: u64,
        );
    }
    struct Handle(*mut c_void);
    impl Drop for Handle {
        fn drop(&mut self) {
            unsafe { player_social_destroy(self.0) };
        }
    }
    pub(super) fn run(
        id: u64,
        receiver: std::sync::mpsc::Receiver<Command>,
        state: Arc<Mutex<SocialStatus>>,
    ) {
        let mut handle: Option<Handle> = None;
        let mut desired = None;
        let mut received = Instant::now();
        let mut dirty = true;
        let mut was_ready = false;
        let mut started = Instant::now();
        let mut published = Instant::now() - Duration::from_secs(10);
        let mut buffer = vec![0_u8; 1024 * 1024];
        loop {
            match receiver.recv_timeout(Duration::from_millis(100)) {
                Ok(Command::Stop(done)) => {
                    drop(handle.take());
                    let _ = done.send(());
                    break;
                }
                Ok(Command::Connect) => {
                    drop(handle.take());
                    let pointer = unsafe { player_social_create(id) };
                    if pointer.is_null() {
                        state.lock().unwrap().error = "Could not start Discord Social SDK.".into();
                    } else {
                        handle = Some(Handle(pointer));
                        started = Instant::now();
                    }
                }
                Ok(Command::Presence(enabled, playback)) => {
                    let removes_key = desired
                        .as_ref()
                        .is_some_and(|p: &PlaybackPresence| p.discovery_public_key.is_some())
                        && playback
                            .as_ref()
                            .is_none_or(|p| p.discovery_public_key.is_none());
                    if !enabled || !playback.as_ref().is_some_and(|p| p.playing) || removes_key {
                        published = Instant::now() - Duration::from_secs(10);
                    }
                    desired = playback.filter(|p| enabled && p.playing);
                    received = Instant::now();
                    dirty = true;
                }
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
                Err(_) => {}
            }
            let Some(handle) = handle.as_ref() else {
                continue;
            };
            let length =
                unsafe { player_social_poll(handle.0, buffer.as_mut_ptr().cast(), buffer.len()) };
            let snapshot: Snapshot = match serde_json::from_slice(&buffer[..length]) {
                Ok(snapshot) => snapshot,
                Err(_) => {
                    state.lock().unwrap().friends.clear();
                    continue;
                }
            };
            let ready = snapshot.connected;
            dirty |= ready && !was_ready;
            was_ready = ready;
            let mut status = matched(snapshot);
            status.connecting =
                !ready && status.error.is_empty() && started.elapsed() < Duration::from_secs(120);
            if !ready && !status.connecting && status.error.is_empty() {
                status.error = "Discord connection timed out. Disconnect and try again.".into();
            }
            *state.lock().unwrap() = status;
            if ready && dirty && published.elapsed() >= Duration::from_secs(5) {
                let p = desired.clone().unwrap_or(PlaybackPresence {
                    title: String::new(),
                    artist: String::new(),
                    album: String::new(),
                    position: 0.,
                    duration: 0.,
                    playing: false,
                    discovery_public_key: None,
                });
                let clean = |s: &str| {
                    let mut value = String::new();
                    for ch in s.chars().filter(|ch| !ch.is_control()) {
                        if value.len() + ch.len_utf8() > 128 {
                            break;
                        }
                        value.push(ch);
                    }
                    CString::new(value).unwrap()
                };
                let title = clean(&p.title);
                let artist = clean(&p.artist);
                let album = clean(&p.album);
                let url = clean(
                    &p.discovery_public_key
                        .map(|key| format!("adhd://v1/{key}"))
                        .unwrap_or_default(),
                );
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as u64;
                let position = if p.position.is_finite() {
                    p.position.max(0.) + received.elapsed().as_secs_f64()
                } else {
                    0.
                };
                let position = if p.duration.is_finite() && p.duration > 0. {
                    position.min(p.duration)
                } else {
                    position
                };
                let start = now.saturating_sub((position * 1000.) as u64);
                let end = if p.duration.is_finite() && p.duration > 0. {
                    Some(start.saturating_add((p.duration * 1000.) as u64))
                } else {
                    None
                };
                unsafe {
                    player_social_presence(
                        handle.0,
                        title.as_ptr(),
                        artist.as_ptr(),
                        album.as_ptr(),
                        url.as_ptr(),
                        p.playing,
                        start,
                        end.unwrap_or(0),
                    )
                };
                dirty = false;
                published = Instant::now();
            }
        }
    }
}

#[tauri::command]
pub fn discord_social_status(
    state: tauri::State<'_, crate::discord_presence::DiscordPresenceState>,
) -> SocialStatus {
    state.social.status()
}

#[tauri::command]
pub fn discord_social_connect(
    state: tauri::State<'_, crate::discord_presence::DiscordPresenceState>,
) -> Result<(), String> {
    state.connect_social()
}

#[tauri::command]
pub fn discord_social_disconnect(
    state: tauri::State<'_, crate::discord_presence::DiscordPresenceState>,
) -> Result<(), String> {
    state.disconnect_social()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn matching_uses_the_same_public_key_namespace_as_local_profiles() {
        let bytes = [7; 32];
        let key = base64::engine::general_purpose::STANDARD.encode(bytes);
        let url = format!("adhd://v1/{key}");
        assert_eq!(
            profile_from_url(&url),
            Some(uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_OID, &bytes).to_string())
        );
        assert!(profile_from_url("adhd://v1/invalid").is_none());
        assert!(profile_from_url(&format!("{url}?other=1")).is_none());
        let user = || SocialUser {
            id: "1".into(),
            name: "A".into(),
            avatar: String::new(),
            url: url.clone(),
            title: "Track".into(),
            artist: String::new(),
            album: String::new(),
        };
        assert_eq!(
            matched(Snapshot {
                connected: true,
                users: vec![user()],
                ..Default::default()
            })
            .friends
            .len(),
            1
        );
        assert!(matched(Snapshot {
            connected: false,
            users: vec![user()],
            ..Default::default()
        })
        .friends
        .is_empty());
        assert!(matched(Snapshot {
            connected: true,
            users: vec![user(), user()],
            ..Default::default()
        })
        .friends
        .is_empty());
    }
}
