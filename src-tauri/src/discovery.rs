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
use tauri::{AppHandle, Emitter, State};

const SERVICE_TYPE: &str = "_player._tcp.local.";
/// Advertise the real sync HTTP port so peers can connect consistently.
const SERVICE_PORT: u16 = crate::sync::SYNC_PORT;

// ── Public types ─────────────────────────────────────────────────────────────

#[derive(Clone, Serialize)]
pub struct Peer {
    pub name: String,  // instance name (human-readable hostname)
    pub host: String,  // resolved hostname or IP
    pub port: u16,
    pub addresses: Vec<String>,
    pub device_name: Option<String>,  // device name from remote /tracks endpoint
    pub device_emoji: Option<String>,  // device emoji from remote /tracks endpoint
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

fn fetch_remote_device_name(host: &str, port: u16) -> (Option<String>, Option<String>) {
    use std::time::Duration;

    let host_for_url = if host.contains(':') && !host.starts_with('[') {
        format!("[{host}]")
    } else {
        host.to_string()
    };
    let url = format!("http://{}:{}/tracks", host_for_url, port);
    
    // Use blocking reqwest with short timeout
    let resp = match reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(2))
        .build()
        .and_then(|client| client.get(&url).send())
    {
        Ok(r) => r,
        Err(_) => return (None, None),
    };
    
    match resp.json::<serde_json::Value>() {
        Ok(val) => {
            let device_name = val.get("device_name")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            let device_emoji = val.get("device_emoji")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            (device_name, device_emoji)
        },
        Err(_) => (None, None),
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
        let mut last_browse_time = std::time::Instant::now();
        let mut receiver = match daemon_clone.browse(SERVICE_TYPE) {
            Ok(r) => r,
            Err(_) => return,
        };

        loop {
            // Try to receive an event with a 5 second timeout
            let event = match receiver.recv_timeout(std::time::Duration::from_secs(5)) {
                Ok(event) => event,
                Err(_) => {
                    // Timeout occurred - periodically restart browse to rediscover peers
                    if last_browse_time.elapsed() > std::time::Duration::from_secs(15) {
                        if let Ok(new_receiver) = daemon_clone.browse(SERVICE_TYPE) {
                            receiver = new_receiver;
                            last_browse_time = std::time::Instant::now();
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

                    // Try to fetch device_name and device_emoji from remote /tracks endpoint
                    let (device_name, device_emoji) = fetch_remote_device_name(&host, port);

                    let peer = Peer {
                        name: hostname,
                        host,
                        port,
                        addresses: addrs,
                        device_name,
                        device_emoji,
                    };
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
