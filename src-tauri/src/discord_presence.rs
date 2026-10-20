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
    #[cfg(all(
        feature = "discord-rpc",
        any(target_os = "macos", target_os = "windows", target_os = "linux")
    ))]
    worker: desktop::Worker,
}

impl DiscordPresenceState {
    pub fn shutdown(&self) {
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
    playback: Option<PlaybackPresence>,
    state: tauri::State<'_, DiscordPresenceState>,
) -> Result<(), String> {
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
            let album = label(&playback.album, "Player");
            let mut timestamps = activity::Timestamps::new().start(start);
            if let Some(end) = end {
                timestamps = timestamps.end(end);
            }
            let presence = activity::Activity::new()
                .activity_type(activity::ActivityType::Listening)
                .details(title)
                .state(artist)
                .timestamps(timestamps)
                .assets(
                    activity::Assets::new()
                        .large_image("player")
                        .large_text(album),
                );
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
    }
    impl<T: Transport> Engine<T> {
        fn new(transport: T) -> Self {
            Self {
                transport,
                connected: false,
                last_version: None,
                last_sent: None,
                retry_at: None,
            }
        }
        fn disconnect(&mut self) {
            if self.connected {
                self.transport.clear();
            }
            self.connected = false;
            self.last_version = None;
            self.last_sent = None;
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
                }),
                received: now,
                version: 1,
                shutdown: false,
            }
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
