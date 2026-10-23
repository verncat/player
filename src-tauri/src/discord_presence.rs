//! Optional Discord adapter. Receives playback snapshots and owns no player state.
use serde::{Deserialize, Serialize};

// Public Discord application identifier. Forks can override it at build time.
const PLAYER_CLIENT_ID: &str = "1562037965537157120";

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct PlaybackPresence {
    pub title: String,
    pub artist: String,
    pub album: String,
    pub position: f64,
    pub duration: f64,
    pub playing: bool,
    // Set natively from the local profile, never accepted from a playback snapshot.
    #[serde(skip_deserializing)]
    pub discovery_public_key: Option<String>,
}

#[derive(Serialize)]
pub struct DiscordAvailability {
    supported: bool,
    configured: bool,
}

fn client_id() -> Option<&'static str> {
    option_env!("DISCORD_CLIENT_ID")
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .or(Some(PLAYER_CLIENT_ID))
        .filter(|id| !id.is_empty() && id.bytes().all(|byte| byte.is_ascii_digit()))
}

const SUPPORTED: bool = cfg!(all(
    feature = "discord-rpc",
    any(
        target_os = "macos",
        target_os = "windows",
        target_os = "linux"
    )
));

#[derive(Default)]
pub struct DiscordPresenceState {
    pub social: crate::discord_social::SocialState,
    latest: std::sync::Mutex<LatestPresence>,
    #[cfg(all(
        feature = "discord-rpc",
        any(target_os = "macos", target_os = "windows", target_os = "linux")
    ))]
    worker: desktop::Worker,
}

#[derive(Default)]
struct LatestPresence {
    enabled: bool,
    playback: Option<PlaybackPresence>,
    received: Option<std::time::Instant>,
}
impl LatestPresence {
    fn snapshot(&self) -> (bool, Option<PlaybackPresence>) {
        let mut playback = self.playback.clone();
        if let Some(p) = playback.as_mut().filter(|p| p.playing) {
            p.position += self
                .received
                .map(|at| at.elapsed().as_secs_f64())
                .unwrap_or_default();
            if p.duration.is_finite() && p.duration > 0. {
                p.position = p.position.min(p.duration);
            }
        }
        (self.enabled, playback)
    }
}

impl DiscordPresenceState {
    pub fn connect_social(&self) -> Result<(), String> {
        if !cfg!(discord_social_sdk) {
            return Err("This build does not include the Discord Social SDK.".into());
        }
        let id = client_id()
            .and_then(|id| id.parse().ok())
            .ok_or("Discord application is not configured")?;
        // One adapter owns Rich Presence at a time. Connecting the SDK can clear legacy RPC.
        #[cfg(all(
            feature = "discord-rpc",
            any(target_os = "macos", target_os = "windows", target_os = "linux")
        ))]
        if let Some(id) = client_id() {
            self.worker.update(id, false, None)?;
        }
        self.social.connect(id)?;
        let (enabled, playback) = self.latest.lock().map_err(|e| e.to_string())?.snapshot();
        self.social.update(enabled, playback);
        Ok(())
    }
    pub fn disconnect_social(&self) -> Result<(), String> {
        self.social.disconnect();
        #[cfg(all(
            feature = "discord-rpc",
            any(target_os = "macos", target_os = "windows", target_os = "linux")
        ))]
        if let Some(id) = client_id() {
            let (enabled, playback) = self.latest.lock().map_err(|e| e.to_string())?.snapshot();
            self.worker.update(id, enabled, playback)?;
        }
        Ok(())
    }
    pub fn shutdown(&self) {
        self.social.disconnect();
        #[cfg(all(
            feature = "discord-rpc",
            any(target_os = "macos", target_os = "windows", target_os = "linux")
        ))]
        self.worker.shutdown();
    }
}

#[tauri::command]
pub fn discord_availability() -> DiscordAvailability {
    DiscordAvailability {
        supported: SUPPORTED,
        configured: SUPPORTED && client_id().is_some(),
    }
}

#[tauri::command]
pub fn discord_update_presence(
    enabled: bool,
    mut playback: Option<PlaybackPresence>,
    discovery_enabled: Option<bool>,
    profile: tauri::State<'_, crate::profile::LocalProfileState>,
    state: tauri::State<'_, DiscordPresenceState>,
) -> Result<(), String> {
    if let Some(playback) = playback.as_mut() {
        playback.discovery_public_key = if enabled && discovery_enabled.unwrap_or(false) {
            Some(profile.info().public_key)
        } else {
            None
        };
    }
    *state.latest.lock().map_err(|e| e.to_string())? = LatestPresence {
        enabled,
        playback: playback.clone(),
        received: Some(std::time::Instant::now()),
    };
    if state
        .social
        .active
        .load(std::sync::atomic::Ordering::SeqCst)
    {
        state.social.update(enabled, playback);
        return Ok(());
    }
    #[cfg(all(
        feature = "discord-rpc",
        any(target_os = "macos", target_os = "windows", target_os = "linux")
    ))]
    {
        if let Some(id) = client_id() {
            return state.worker.update(id, enabled, playback);
        }
    }
    let _ = (playback, state);
    if enabled {
        Err("Discord Rich Presence is not available in this build.".into())
    } else {
        Ok(())
    }
}

#[cfg(all(
    feature = "discord-rpc",
    any(target_os = "macos", target_os = "windows", target_os = "linux")
))]
mod desktop {
    use super::PlaybackPresence;
    use discord_rich_presence::{activity, DiscordIpc, DiscordIpcClient};
    use std::sync::{mpsc, Arc, Mutex};
    use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

    const UPDATE_INTERVAL: Duration = Duration::from_secs(5);
    const RETRY_INTERVAL: Duration = Duration::from_secs(15);
    const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(30);

    #[derive(Clone)]
    struct Desired {
        enabled: bool,
        playback: Option<PlaybackPresence>,
        received: Instant,
        version: u64,
        shutdown: bool,
    }

    impl Default for Desired {
        fn default() -> Self {
            Self {
                enabled: false,
                playback: None,
                received: Instant::now(),
                version: 0,
                shutdown: false,
            }
        }
    }

    #[derive(Default)]
    pub(super) struct Worker {
        desired: Arc<Mutex<Desired>>,
        wake: Mutex<Option<mpsc::SyncSender<()>>>,
    }

    impl Worker {
        pub fn update(
            &self,
            id: &'static str,
            enabled: bool,
            playback: Option<PlaybackPresence>,
        ) -> Result<(), String> {
            let mut desired = self.desired.lock().map_err(|e| e.to_string())?;
            desired.enabled = enabled;
            desired.playback = playback;
            desired.received = Instant::now();
            desired.version = desired.version.wrapping_add(1);
            drop(desired);
            let mut wake = self.wake.lock().map_err(|e| e.to_string())?;
            if wake.is_none() && enabled {
                let (tx, rx) = mpsc::sync_channel(1);
                let shared = self.desired.clone();
                std::thread::Builder::new()
                    .name("discord-presence".into())
                    .spawn(move || {
                        let mut engine = Engine::new(DiscordTransport::new(id));
                        loop {
                            let desired = match shared.lock() {
                                Ok(value) => value.clone(),
                                Err(_) => break,
                            };
                            if desired.shutdown {
                                break;
                            }
                            engine.tick(&desired, Instant::now(), unix_millis());
                            let wait = if desired.enabled {
                                Duration::from_secs(1)
                            } else {
                                Duration::from_secs(60)
                            };
                            if matches!(
                                rx.recv_timeout(wait),
                                Err(mpsc::RecvTimeoutError::Disconnected)
                            ) {
                                break;
                            }
                        }
                        engine.disconnect();
                    })
                    .map_err(|e| e.to_string())?;
                *wake = Some(tx);
            }
            if let Some(wake) = wake.as_ref() {
                let _ = wake.try_send(());
            }
            Ok(())
        }

        pub fn shutdown(&self) {
            if let Ok(mut desired) = self.desired.lock() {
                desired.shutdown = true;
            }
            if let Ok(mut wake) = self.wake.lock() {
                wake.take();
            }
        }
    }

    trait Transport {
        fn connect(&mut self) -> Result<(), String>;
        fn publish(
            &mut self,
            playback: &PlaybackPresence,
            start: i64,
            end: Option<i64>,
        ) -> Result<(), String>;
        fn clear(&mut self);
    }

    struct DiscordTransport {
        id: &'static str,
        client: Option<DiscordIpcClient>,
    }
    impl DiscordTransport {
        fn new(id: &'static str) -> Self {
            Self { id, client: None }
        }
        fn acknowledge(client: &mut DiscordIpcClient) -> Result<(), String> {
            let (opcode, response) = client.recv().map_err(|e| e.to_string())?;
            if opcode != 1 || response["evt"] == "ERROR" {
                return Err("Discord rejected the activity".into());
            }
            Ok(())
        }
    }
    impl Transport for DiscordTransport {
        fn connect(&mut self) -> Result<(), String> {
            let mut client = DiscordIpcClient::new(self.id);
            client.connect_ipc().map_err(|e| e.to_string())?;
            client
                .send(serde_json::json!({ "v": 1, "client_id": self.id }), 0)
                .map_err(|e| e.to_string())?;
            let (opcode, response) = client.recv().map_err(|e| e.to_string())?;
            if opcode != 1 || response["evt"] != "READY" {
                return Err("Discord handshake rejected".into());
            }
            self.client = Some(client);
            Ok(())
        }
        fn publish(
            &mut self,
            playback: &PlaybackPresence,
            start: i64,
            end: Option<i64>,
        ) -> Result<(), String> {
            let client = self.client.as_mut().ok_or("Discord is not connected")?;
            let title = label(&playback.title, "Unknown track");
            let artist = label(&playback.artist, "Unknown artist");
            let mut timestamps = activity::Timestamps::new().start(start);
            if let Some(end) = end {
                timestamps = timestamps.end(end);
            }
            let presence = activity::Activity::new()
                .activity_type(activity::ActivityType::Listening)
                .details(title)
                .state(artist)
                .timestamps(timestamps)
                .assets(activity_assets(playback));
            client.set_activity(presence).map_err(|e| e.to_string())?;
            Self::acknowledge(client)
        }
        fn clear(&mut self) {
            if let Some(mut client) = self.client.take() {
                let _ = client.clear_activity();
                let _ = client.close();
            }
        }
    }

    fn label(value: &str, fallback: &str) -> String {
        let clean: String = value.chars().filter(|c| !c.is_control()).collect();
        let clean = clean.trim();
        let value = if clean.is_empty() { fallback } else { clean };
        // Discord limits these fields to 128 bytes; keep UTF-8 boundaries intact.
        let mut end = value.len().min(128);
        while !value.is_char_boundary(end) {
            end -= 1;
        }
        value[..end].into()
    }

    fn activity_assets(playback: &PlaybackPresence) -> activity::Assets<'static> {
        let assets = activity::Assets::new()
            .large_image("player")
            .large_text(label(&playback.album, "Player"));
        match &playback.discovery_public_key {
            Some(key) => assets.small_url(format!("adhd://v1/{key}")),
            None => assets,
        }
    }

    fn unix_millis() -> i64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis()
            .min(i64::MAX as u128) as i64
    }

    fn timestamps(playback: &PlaybackPresence, elapsed: Duration, now: i64) -> (i64, Option<i64>) {
        let seconds = |value: f64| {
            if value.is_finite() {
                value.clamp(0.0, 31_536_000.0)
            } else {
                0.0
            }
        };
        let duration = seconds(playback.duration);
        let position = seconds(playback.position) + elapsed.as_secs_f64();
        let position = if duration > 0.0 {
            position.min(duration)
        } else {
            position
        };
        let start = now.saturating_sub((position * 1000.0) as i64).max(0);
        let end = (duration > 0.0).then(|| start.saturating_add((duration * 1000.0) as i64));
        (start, end)
    }

    struct Engine<T> {
        transport: T,
        connected: bool,
        last_version: Option<u64>,
        last_sent: Option<Instant>,
        retry_at: Option<Instant>,
        last_discovery_key: Option<String>,
    }
    impl<T: Transport> Engine<T> {
        fn new(transport: T) -> Self {
            Self {
                transport,
                connected: false,
                last_version: None,
                last_sent: None,
                retry_at: None,
                last_discovery_key: None,
            }
        }
        fn disconnect(&mut self) {
            if self.connected {
                self.transport.clear();
            }
            self.connected = false;
            self.last_version = None;
            self.last_sent = None;
            self.last_discovery_key = None;
        }
        fn tick(&mut self, desired: &Desired, now: Instant, wall: i64) {
            let playback = desired
                .playback
                .as_ref()
                .filter(|p| desired.enabled && p.playing);
            let Some(playback) = playback else {
                self.disconnect();
                self.retry_at = None;
                return;
            };
            // Remove an advertised key immediately, even inside the normal update throttle.
            if self.last_discovery_key.is_some() && playback.discovery_public_key.is_none() {
                self.disconnect();
            }
            if self.retry_at.is_some_and(|retry| now < retry) {
                return;
            }
            let result = (|| {
                if !self.connected {
                    self.transport.connect()?;
                    self.connected = true;
                }
                if let Some(last) = self.last_sent {
                    let age = now.duration_since(last);
                    if age < UPDATE_INTERVAL
                        || (self.last_version == Some(desired.version) && age < HEARTBEAT_INTERVAL)
                    {
                        return Ok(());
                    }
                }
                let (start, end) = timestamps(playback, now.duration_since(desired.received), wall);
                self.transport.publish(playback, start, end)?;
                self.last_version = Some(desired.version);
                self.last_sent = Some(now);
                self.last_discovery_key = playback.discovery_public_key.clone();
                self.retry_at = None;
                Ok::<_, String>(())
            })();
            if let Err(error) = result {
                tracing::debug!(%error, "Discord presence unavailable; will retry");
                self.disconnect();
                self.retry_at = Some(now + RETRY_INTERVAL);
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        #[derive(Default)]
        struct Fake {
            connects: usize,
            publishes: usize,
            clears: usize,
            fail: bool,
            last_title: String,
            last_large_text: String,
            last_small_url: Option<String>,
        }
        impl Transport for Fake {
            fn connect(&mut self) -> Result<(), String> {
                self.connects += 1;
                if self.fail {
                    Err("offline".into())
                } else {
                    Ok(())
                }
            }
            fn publish(
                &mut self,
                p: &PlaybackPresence,
                _: i64,
                _: Option<i64>,
            ) -> Result<(), String> {
                self.publishes += 1;
                self.last_title = p.title.clone();
                let assets = serde_json::to_value(activity_assets(p)).unwrap();
                self.last_large_text = assets["large_text"].as_str().unwrap().into();
                self.last_small_url = assets
                    .get("small_url")
                    .and_then(|url| url.as_str())
                    .map(str::to_owned);
                if self.fail {
                    Err("disconnected".into())
                } else {
                    Ok(())
                }
            }
            fn clear(&mut self) {
                self.clears += 1;
            }
        }
        fn desired(now: Instant) -> Desired {
            Desired {
                enabled: true,
                playback: Some(PlaybackPresence {
                    title: "Song".into(),
                    artist: "Artist".into(),
                    album: "Album".into(),
                    position: 30.0,
                    duration: 180.0,
                    playing: true,
                    discovery_public_key: None,
                }),
                received: now,
                version: 1,
                shutdown: false,
            }
        }
        #[test]
        fn discovery_metadata_uses_the_versioned_public_key_format() {
            let mut d = desired(Instant::now());
            let playback = d.playback.as_mut().unwrap();
            let assets = serde_json::to_value(activity_assets(playback)).unwrap();
            assert_eq!(assets["large_text"], "Album");
            assert!(assets.get("small_url").is_none());
            let key = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=";
            playback.discovery_public_key = Some(key.into());
            let activity = activity::Activity::new().assets(activity_assets(playback));
            let json = serde_json::to_value(activity).unwrap();
            assert_eq!(json["assets"]["small_url"], format!("adhd://v1/{key}"));
            assert_eq!(json["assets"]["large_text"], "Album");
            playback.discovery_public_key = None;
            playback.album.clear();
            let assets = serde_json::to_value(activity_assets(playback)).unwrap();
            assert_eq!(assets["large_text"], "Player");
            assert!(assets.get("small_url").is_none());
        }

        #[test]
        fn playback_payload_cannot_inject_a_discovery_key() {
            let payload = serde_json::json!({
                "title": "Song", "artist": "Artist", "album": "Album",
                "position": 0, "duration": 10, "playing": true,
                "discovery_public_key": "untrusted"
            });
            let playback: PlaybackPresence = serde_json::from_value(payload).unwrap();
            assert!(playback.discovery_public_key.is_none());
            let assets = serde_json::to_value(activity_assets(&playback)).unwrap();
            assert_eq!(assets["large_text"], "Album");
            assert!(assets.get("small_url").is_none());
        }

        #[test]
        fn disabling_discovery_removes_the_key_without_waiting_for_the_throttle() {
            let now = Instant::now();
            let mut d = desired(now);
            d.playback.as_mut().unwrap().discovery_public_key = Some("public-key".into());
            let mut engine = Engine::new(Fake::default());
            engine.tick(&d, now, 100_000);
            assert_eq!(
                engine.transport.last_small_url.as_deref(),
                Some("adhd://v1/public-key")
            );
            assert_eq!(engine.transport.last_large_text, "Album");
            d.version += 1;
            d.playback.as_mut().unwrap().discovery_public_key = None;
            engine.tick(&d, now + Duration::from_secs(1), 101_000);
            assert_eq!(engine.transport.clears, 1);
            assert_eq!(engine.transport.publishes, 2);
            assert_eq!(engine.transport.last_large_text, "Album");
            assert!(engine.transport.last_small_url.is_none());
        }

        #[test]
        fn disabled_never_connects_and_pause_clears_immediately() {
            let now = Instant::now();
            let mut d = desired(now);
            let mut e = Engine::new(Fake::default());
            d.enabled = false;
            e.tick(&d, now, 100_000);
            assert_eq!(e.transport.connects, 0);
            d.enabled = true;
            e.tick(&d, now, 100_000);
            assert_eq!(e.transport.publishes, 1);
            d.playback.as_mut().unwrap().playing = false;
            e.tick(&d, now, 100_000);
            assert_eq!(e.transport.clears, 1);
        }
        #[test]
        fn updates_are_coalesced_but_disable_bypasses_throttle() {
            let now = Instant::now();
            let mut d = desired(now);
            let mut e = Engine::new(Fake::default());
            e.tick(&d, now, 100_000);
            d.version = 2;
            d.playback.as_mut().unwrap().title = "New song".into();
            e.tick(&d, now + Duration::from_secs(1), 101_000);
            assert_eq!(e.transport.publishes, 1);
            e.tick(&d, now + UPDATE_INTERVAL, 105_000);
            assert_eq!(e.transport.last_title, "New song");
            d.enabled = false;
            e.tick(&d, now + UPDATE_INTERVAL, 105_000);
            assert_eq!(e.transport.clears, 1);
        }
        #[test]
        fn reconnects_with_latest_track_after_discord_restarts() {
            let now = Instant::now();
            let mut d = desired(now);
            let mut e = Engine::new(Fake {
                fail: true,
                ..Fake::default()
            });
            e.tick(&d, now, 100_000);
            assert_eq!(e.transport.connects, 1);
            e.transport.fail = false;
            d.playback.as_mut().unwrap().title = "Latest".into();
            d.version = 2;
            e.tick(&d, now + Duration::from_secs(1), 101_000);
            assert_eq!(e.transport.connects, 1);
            e.tick(&d, now + RETRY_INTERVAL, 115_000);
            assert_eq!(e.transport.connects, 2);
            assert_eq!(e.transport.last_title, "Latest");
            e.transport.fail = true;
            e.tick(&d, now + RETRY_INTERVAL + HEARTBEAT_INTERVAL, 145_000);
            assert!(!e.connected);
        }
        #[test]
        fn timestamps_account_for_seek_delay_and_unknown_duration() {
            let now = Instant::now();
            let mut p = desired(now).playback.unwrap();
            assert_eq!(
                timestamps(&p, Duration::from_secs(5), 100_000),
                (65_000, Some(245_000))
            );
            p.position = 90.0;
            assert_eq!(
                timestamps(&p, Duration::ZERO, 100_000),
                (10_000, Some(190_000))
            );
            p.duration = f64::NAN;
            p.position = f64::INFINITY;
            assert_eq!(timestamps(&p, Duration::ZERO, 100_000), (100_000, None));
        }
        #[test]
        fn unicode_metadata_is_limited_without_breaking_utf8() {
            let text = label(&"曲".repeat(100), "Unknown");
            assert!(text.len() <= 128);
            assert_eq!(text.chars().count(), 42);
            assert_eq!(label("\n\t", "Unknown track"), "Unknown track");
        }
    }
}
