use std::collections::VecDeque;
use std::sync::Arc;

use crate::actor::server_actor::ServerCommand;
use crate::actor::{ActorHandle, ActorSystem};
use crate::client::ClientOperation;
use crate::types::Download;
use tokio::sync::mpsc::UnboundedSender;

pub enum ClientState {
    Disconnected,
    Connecting,
    Connected,
}

/// Holds all live-connection resources. Created on connect, persists across disconnects
/// (ServerActor handles auto-reconnect). Only cleared when a new connect() is initiated.
pub struct ActiveConnection {
    pub server_handle: ActorHandle<ServerCommand>,
    /// Sender to the ConnectedWorker operations channel.
    pub op_tx: UnboundedSender<ClientOperation>,
    /// Actor system — used for shutdown.
    pub actor_system: Arc<ActorSystem>,
}

/// All mutable state behind a single lock.
pub struct ClientInner {
    pub state: ClientState,
    /// Present after the first successful connect(); persists across disconnects.
    pub active: Option<ActiveConnection>,
    /// Downloads queued before connect() is ever called; seeded into the worker on first connect.
    pub pending_downloads: VecDeque<Download>,
}

impl Drop for ClientInner {
    fn drop(&mut self) {
        if let Some(active) = &self.active {
            active.actor_system.shutdown();
        }
    }
}
