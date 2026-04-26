//! mDNS/DNS-SD peer discovery.
//!
//! Each running instance:
//!   1. Registers itself as `_player._tcp.local.` so others can find it.
//!   2. Browses for the same service type and emits `"discovery-peers"` Tauri
//!      events whenever the peer list changes.
//!
//! Tauri commands exposed:
//!   `discovery_start`  — start advertising + browsing (idempotent).
//!   `discovery_stop`   — stop everything.
//!   `discovery_peers`  — return current snapshot of known peers.

use mdns_sd::{ServiceDaemon, ServiceEvent, ServiceInfo};
use serde::Serialize;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, State};

const SERVICE_TYPE: &str = "_player._tcp.local.";
/// Advertise the real sync HTTP port so peers can connect consistently.
const SERVICE_PORT: u16 = crate::sync::SYNC_PORT;
const PEER_STATUS_REQUEST_TIMEOUT: Duration = Duration::from_millis(1200);
const PEER_STATUS_REFRESH_INTERVAL: Duration = Duration::from_secs(5);
const BROWSE_EVENT_TIMEOUT: Duration = Duration::from_secs(5);
const BROWSE_RESTART_INTERVAL: Duration = Duration::from_secs(15);

// ── Public types ─────────────────────────────────────────────────────────────

#[derive(Clone, Serialize, PartialEq, Eq)]
pub struct Peer {
    pub name: String,  // instance name (human-readable hostname)
    pub host: String,  // resolved hostname or IP
    pub port: u16,
    pub addresses: Vec<String>,
    pub device_name: Option<String>,  // device name from remote /status endpoint
    pub device_emoji: Option<String>,  // device emoji from remote /status endpoint
    pub playback: Option<crate::sync::RemotePlaybackInfo>,
}

// ── State ─────────────────────────────────────────────────────────────────────

pub struct DiscoveryState {
    inner: Arc<Mutex<Inner>>,
}

struct Inner {
    daemon: Option<ServiceDaemon>,
    peers: HashMap<String, Peer>,
    own_instance: String,
}

impl DiscoveryState {
    pub fn new() -> Self {
        let instance = own_instance_name();
        DiscoveryState {
            inner: Arc::new(Mutex::new(Inner {
                daemon: None,
                peers: HashMap::new(),
                own_instance: instance,
            })),
        }
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn own_instance_name() -> String {
    let name = hostname::get()
        .ok()
        .and_then(|h| h.into_string().ok())
        .unwrap_or_else(|| "Player".to_string());
    // On Android, hostname::get() often returns "localhost" which is useless
    // for mDNS. Fall back to whoami::devicename() for a real device name.
    if name == "localhost" {
        let dev = whoami::devicename();
        if !dev.is_empty() && dev != "localhost" {
            return dev;
        }
        return "Android-Player".to_string();
    }
    name
}

fn peer_status_url(host: &str, port: u16, path: &str) -> String {
    let host_for_url = if host.contains(':') && !host.starts_with('[') {
        format!("[{host}]")
    } else {
        host.to_string()
    };
    format!("http://{}:{}{}", host_for_url, port, path)
}

fn fetch_remote_status(host: &str, port: u16) -> Option<crate::sync::DeviceStatusResponse> {
    let client = reqwest::blocking::Client::builder()
        .timeout(PEER_STATUS_REQUEST_TIMEOUT)
        .build()
        .ok()?;

    let status_url = peer_status_url(host, port, "/status");
    if let Ok(resp) = client.get(&status_url).send() {
        if let Ok(status) = resp.json::<crate::sync::DeviceStatusResponse>() {
            return Some(status);
        }
    }

    let tracks_url = peer_status_url(host, port, "/tracks");
    if let Ok(resp) = client.get(&tracks_url).send() {
        if let Ok(status) = resp.json::<crate::sync::TracksResponse>() {
            return Some(crate::sync::DeviceStatusResponse {
                version: status.version,
                device_name: status.device_name,
                device_emoji: status.device_emoji,
                playback: None,
            });
        }
    }

    None
}

fn apply_remote_status(peer: &Peer, status: crate::sync::DeviceStatusResponse) -> Peer {
    let mut updated = peer.clone();
    if status.device_name.is_some() {
        updated.device_name = status.device_name;
    }
    if status.device_emoji.is_some() {
        updated.device_emoji = status.device_emoji;
    }
    updated.playback = status.playback;
    updated
}

fn refresh_peer_statuses(app: &AppHandle, peers_arc: &Arc<Mutex<Inner>>) {
    let snapshot: Vec<(String, Peer)> = {
        let guard = peers_arc.lock().unwrap();
        guard
            .peers
            .iter()
            .map(|(key, peer)| (key.clone(), peer.clone()))
            .collect()
    };
    let mut updates = Vec::new();
    for (key, peer) in snapshot {
        let Some(status) = fetch_remote_status(&peer.host, peer.port) else {
            continue;
        };
        let updated = apply_remote_status(&peer, status);
        if updated != peer {
            updates.push((key, updated));
        }
    }
    if updates.is_empty() {
        return;
    }
    let mut guard = peers_arc.lock().unwrap();
    let mut changed = false;
    for (key, updated) in updates {
        if let Some(existing) = guard.peers.get_mut(&key) {
            if *existing != updated {
                *existing = updated;
                changed = true;
            }
        }
    }
    if changed {
        emit_peers(app, &guard.peers);
    }
}

// ── Commands ──────────────────────────────────────────────────────────────────

#[tauri::command]
pub fn discovery_start(
    state: State<'_, DiscoveryState>,
    app: AppHandle,
) -> Result<(), String> {
    let mut inner = state.inner.lock().unwrap();
    if inner.daemon.is_some() {
        return Ok(()); // already running
    }

    let daemon = ServiceDaemon::new().map_err(|e| e.to_string())?;

    // ── Register own service ─────────────────────────────────────────────────
    let instance_name = inner.own_instance.clone();
    // Strip .local suffix from hostname to avoid double .local (e.g. "nica-mac-2.local" → "nica-mac-2")
    let base_name = instance_name
        .strip_suffix(".local")
        .unwrap_or(&instance_name);
    let host_name = format!("{base_name}-player.local.");
    let service = ServiceInfo::new(
        SERVICE_TYPE,
        &instance_name,
        &host_name,
        "",           // leave IP to the daemon
        SERVICE_PORT,
        None,         // no extra properties
    )
    .map_err(|e| e.to_string())?
    .enable_addr_auto();

    daemon.register(service).map_err(|e| e.to_string())?;

    let peers_arc = Arc::clone(&state.inner);
    let own = instance_name.clone();
    let daemon_clone = daemon.clone();
    
    std::thread::spawn(move || {
        let mut last_browse_time = Instant::now();
        let mut last_status_refresh = Instant::now();
        let mut receiver = match daemon_clone.browse(SERVICE_TYPE) {
            Ok(r) => r,
            Err(_) => return,
        };

        loop {
            // Try to receive an event with a 5 second timeout
            let event = match receiver.recv_timeout(BROWSE_EVENT_TIMEOUT) {
                Ok(event) => event,
                Err(_) => {
                    if last_status_refresh.elapsed() >= PEER_STATUS_REFRESH_INTERVAL {
                        refresh_peer_statuses(&app, &peers_arc);
                        last_status_refresh = Instant::now();
                    }
                    // Timeout occurred - periodically restart browse to rediscover peers
                    if last_browse_time.elapsed() > BROWSE_RESTART_INTERVAL {
                        if let Ok(new_receiver) = daemon_clone.browse(SERVICE_TYPE) {
                            receiver = new_receiver;
                            last_browse_time = Instant::now();
                        }
                    }
                    continue;
                }
            };

            match event {
                ServiceEvent::ServiceResolved(info) => {
                    let name = info.get_fullname().to_string();
                    // Skip ourselves
                    if name.starts_with(&own) {
                        continue;
                    }
                    let raw_addrs = info.get_addresses();
                    let addrs: Vec<String> = raw_addrs.iter().map(|a| a.to_string()).collect();
                    let hostname = info.get_hostname().trim_end_matches('.').to_string();
                    let port = info.get_port();
                    // Use a pre-resolved IP as host so every subsequent connection
                    // skips mDNS re-resolution entirely. Fall back to the .local
                    // hostname only when no IP was returned by the daemon.
                    let host = best_addr(raw_addrs).unwrap_or_else(|| hostname.clone());

                    let remote_status = fetch_remote_status(&host, port);

                    let peer = apply_remote_status(&Peer {
                        name: hostname,
                        host,
                        port,
                        addresses: addrs,
                        device_name: None,
                        device_emoji: None,
                        playback: None,
                    }, remote_status.unwrap_or(crate::sync::DeviceStatusResponse {
                        version: String::new(),
                        device_name: None,
                        device_emoji: None,
                        playback: None,
                    }));
                    {
                        let mut g = peers_arc.lock().unwrap();
                        g.peers.insert(name, peer);
                        emit_peers(&app, &g.peers);
                    }
                }
                ServiceEvent::ServiceRemoved(_, fullname) => {
                    let mut g = peers_arc.lock().unwrap();
                    g.peers.remove(&fullname);
                    emit_peers(&app, &g.peers);
                }
                ServiceEvent::SearchStopped(_) => break,
                _ => {}
            }
        }
    });

    inner.daemon = Some(daemon);
    Ok(())
}

#[tauri::command]
pub fn discovery_stop(state: State<'_, DiscoveryState>) -> Result<(), String> {
    let mut inner = state.inner.lock().unwrap();
    if let Some(daemon) = inner.daemon.take() {
        let _ = daemon.shutdown();
    }
    inner.peers.clear();
    Ok(())
}

#[tauri::command]
pub fn discovery_peers(state: State<'_, DiscoveryState>) -> Vec<Peer> {
    state
        .inner
        .lock()
        .unwrap()
        .peers
        .values()
        .cloned()
        .collect()
}

fn emit_peers(app: &AppHandle, peers: &HashMap<String, Peer>) {
    let list: Vec<&Peer> = peers.values().collect();
    let _ = app.emit("discovery-peers", list);
}

/// Pick the most connectable IP from a set of resolved addresses.
/// Prefers routable IPv4; falls back to non-link-local IPv6 (raw, no brackets).
fn best_addr(addrs: &std::collections::HashSet<std::net::IpAddr>) -> Option<String> {
    if let Some(ip) = addrs.iter().find(|a| matches!(a, std::net::IpAddr::V4(_))) {
        return Some(ip.to_string());
    }
    addrs.iter().find(|a| {
        if let std::net::IpAddr::V6(v6) = a { !v6.is_unicast_link_local() } else { false }
    }).map(|ip| ip.to_string())
}
