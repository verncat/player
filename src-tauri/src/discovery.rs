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
use tauri::{AppHandle, Emitter, Manager, State};

const SERVICE_TYPE: &str = "_player._tcp.local.";
/// Port we advertise — does not need to be a real HTTP server for discovery.
const SERVICE_PORT: u16 = 57321;

// ── Public types ─────────────────────────────────────────────────────────────

#[derive(Clone, Serialize)]
pub struct Peer {
    pub name: String,  // instance name (human-readable hostname)
    pub host: String,  // resolved hostname or IP
    pub port: u16,
    pub addresses: Vec<String>,
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
    hostname::get()
        .ok()
        .and_then(|h| h.into_string().ok())
        .unwrap_or_else(|| "Player".to_string())
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
    let host_name = format!("{}-player.local.", instance_name);
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

    // ── Browse for peers ─────────────────────────────────────────────────────
    let receiver = daemon.browse(SERVICE_TYPE).map_err(|e| e.to_string())?;

    let peers_arc = Arc::clone(&state.inner);
    let own = instance_name.clone();
    std::thread::spawn(move || {
        while let Ok(event) = receiver.recv() {
            match event {
                ServiceEvent::ServiceResolved(info) => {
                    let name = info.get_fullname().to_string();
                    // Skip ourselves
                    if name.starts_with(&own) {
                        continue;
                    }
                    let addrs: Vec<String> = info
                        .get_addresses()
                        .iter()
                        .map(|a| a.to_string())
                        .collect();
                    let peer = Peer {
                        name: info.get_hostname().trim_end_matches('.').to_string(),
                        host: addrs.first().cloned().unwrap_or_default(),
                        port: info.get_port(),
                        addresses: addrs,
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
