use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use tokio::sync::mpsc::UnboundedReceiver;
use tokio::sync::mpsc::UnboundedSender;
use tokio_util::sync::CancellationToken;

use super::download_manager::DownloadManager;
use crate::actor::server_actor::ServerCommand;
use crate::actor::ActorHandle;
use crate::client::inner::{ClientInner, ClientState};
use crate::client::{ClientContext, ClientOperation};
use crate::token::{DownloadToken, PeerTransferToken, SearchToken};
use crate::types::{Download, Search};
use crate::{debug, error, trace, warn};
use crate::peer::download_peer::spawn_direct_download;
use crate::peer::{ConnectionType, DownloadPeer, Peer};

/// Owns the incoming-operations loop for a live connection.
/// Handles all `ClientOperation` messages from actors (server, peers).
pub struct ConnectedWorker {
    pub own_username: String,
    /// Sender half — cloned into spawned closures so they can send back operations.
    pub op_tx: UnboundedSender<ClientOperation>,
    pub op_rx: UnboundedReceiver<ClientOperation>,
    /// Shared client state — worker mutates `.state` directly on connect/disconnect.
    pub inner: Arc<Mutex<ClientInner>>,
    pub context: Arc<ClientContext>,
    pub cancellation_token: CancellationToken,
    /// Handle to ServerActor — used to send commands (PierceFirewall, GetPeerAddress).
    pub server_handle: ActorHandle<ServerCommand>,
    /// All download lifecycle logic.
    pub downloads: DownloadManager,
    /// All active searches keyed by token.
    pub searches: HashMap<SearchToken, Search>,
}

impl ConnectedWorker {
    pub async fn run(mut self) {
        loop {
            tokio::select! {
                _ = self.cancellation_token.cancelled() => {
                    trace!("[worker] Shutdown signal received");
                    break;
                }
                op = self.op_rx.recv() => {
                    match op {
                        Some(op) => self.handle_operation(op).await,
                        None => {
                            error!("[worker] Channel closed");
                            break;
                        }
                    }
                }
            }
        }
    }

    async fn handle_operation(&mut self, op: ClientOperation) {
        match op {
            // ── Server lifecycle ─────────────────────────────────────────────
            ClientOperation::ServerDisconnected => {
                self.downloads.on_server_disconnected();
                self.inner.lock().unwrap_or_else(|e| e.into_inner()).state =
                    ClientState::Disconnected;
            }
            ClientOperation::LoginSucceeded => {
                self.downloads.on_login_succeeded();
                self.inner.lock().unwrap_or_else(|e| e.into_inner()).state =
                    ClientState::Connected;
            }

            // ── Download events ───────────────────────────────────────────────
            ClientOperation::RequestDownload(download) => {
                self.downloads.enqueue(download);
            }
            ClientOperation::DownloadCompleted(token, result) => {
                self.downloads.on_completed(token, result);
            }
            ClientOperation::CancelDownload(token) => {
                self.downloads.on_cancel(token);
            }
            ClientOperation::DownloadResponseTimeout(token) => {
                self.downloads.on_timeout(token);
            }
            ClientOperation::UpdateDownloadTokens(transfer, username) => {
                self.downloads.on_transfer_request(&transfer, &username);
            }
            ClientOperation::QueuePositionUpdated { username, filename, place } => {
                self.downloads.on_queue_position(&username, &filename, place);
            }
            ClientOperation::TransferRejected { token, .. } => {
                self.downloads.on_transfer_rejected(token);
            }
            ClientOperation::UploadFailed(username, filename) => {
                self.downloads.on_upload_failed(&username, &filename);
            }
            ClientOperation::DownloadFromPeer(peer_transfer_token, peer, _allowed) => {
                let maybe_download = self.downloads.find_by_peer_token(peer_transfer_token).cloned();
                let own_username = self.own_username.clone();
                let op_tx = self.op_tx.clone();

                trace!(
                    "[worker] DownloadFromPeer peer_token: {} peer: {:?}",
                    peer_transfer_token, peer
                );

                match maybe_download {
                    Some(download) => {
                        spawn_direct_download(
                            download,
                            peer.host,
                            peer.port,
                            peer_transfer_token.0,
                            own_username,
                            None,
                            op_tx,
                        );
                    }
                    None => {
                        error!("Can't find download with peer_token {:?}", peer_transfer_token);
                    }
                }
            }

            // ── Peer lifecycle ────────────────────────────────────────────────
            ClientOperation::PeerDisconnected(username, maybe_error) => {
                if let Some(handle) = self.context.peer_registry.remove_peer(&username) {
                    let _ = handle.stop();
                }
                if let Some(ref _error) = maybe_error {
                    warn!(
                        "[worker] Peer {} disconnected with error: {:?}",
                        username, _error
                    );
                }
                self.downloads.on_peer_disconnected(&username);
            }
            ClientOperation::PierceFireWall(peer) => {
                debug!("Piercing firewall for peer: {:?}", peer);
                if let Some(token) = peer.token {
                    if let Err(_e) = self.server_handle.send(ServerCommand::PierceFirewall(token)) {
                        error!("Failed to send PierceFirewall message: {}", _e);
                    }
                } else {
                    error!("No token available for PierceFirewall");
                }
                let initiating_token = self.downloads.find_initiating_token(&peer.username);
                self.peer_connector().connect_f(peer, initiating_token, None);
            }
            ClientOperation::ConnectToPeer(peer) => {
                let connector = self.peer_connector();
                let initiating_token = self.downloads.find_initiating_token(&peer.username);
                tokio::spawn(async move {
                    match peer.connection_type {
                        ConnectionType::P => connector.connect_p(peer, None),
                        ConnectionType::F => connector.connect_f(peer, initiating_token, None),
                        ConnectionType::D => error!("ConnectionType::D not implemented"),
                    }
                });
            }
            ClientOperation::NewPeer(new_peer) => {
                let peer_exists = self.context.peer_registry.contains(&new_peer.username);

                if peer_exists {
                    debug!("Already connected to {}", new_peer.username);
                } else if let Err(_e) = self.server_handle
                    .send(ServerCommand::GetPeerAddress(new_peer.username.clone()))
                {
                    error!("[worker] Failed to send GetPeerAddress: {}", _e);
                }

                let addr = new_peer.tcp_stream.peer_addr().unwrap();
                let host = addr.ip().to_string();
                let port: u32 = addr.port().into();

                let peer = Peer {
                    username: new_peer.username.clone(),
                    connection_type: new_peer.connection_type,
                    host,
                    port,
                    token: Some(new_peer.token),
                    privileged: None,
                    obfuscated_port: None,
                    unknown: None,
                };

                let connector = self.peer_connector();
                let initiating_token = self.downloads.find_initiating_token(&peer.username);
                match peer.connection_type {
                    ConnectionType::P => connector.connect_p(peer, Some(new_peer.tcp_stream)),
                    ConnectionType::F => {
                        connector.connect_f(peer, initiating_token, Some(new_peer.tcp_stream))
                    }
                    ConnectionType::D => error!("ConnectionType::D not implemented"),
                }
                self.downloads.retry_queued_local_for_peer(&new_peer.username);
            }
            ClientOperation::GetPeerAddressResponse {
                username,
                host,
                port,
                obfuscation_type,
                obfuscated_port,
            } => {
                debug!(
                    "Received peer address for {}: {}:{} (obf_type: {}, obf_port: {})",
                    username, host, port, obfuscation_type, obfuscated_port
                );

                let peer_exists = self.context.peer_registry.contains(&username);

                if !peer_exists {
                    let peer = Peer::new(
                        username.clone(),
                        ConnectionType::P,
                        host,
                        port,
                        None,
                        0,
                        obfuscation_type.try_into().unwrap(),
                        obfuscated_port.try_into().unwrap(),
                    );
                    let connector = self.peer_connector();
                    connector.connect_p(peer, None);
                }
                self.downloads.retry_queued_local_for_peer(&username);
            }

            // ── Search events ─────────────────────────────────────────────────
            ClientOperation::SearchResult(search_result) => {
                trace!("[worker] SearchResult {:?}", search_result);
                if let Some(search) = self.searches.get_mut(&search_result.token) {
                    search.results.push(search_result);
                }
            }
            ClientOperation::InitiateSearch(token, query) => {
                self.searches.insert(token, Search { token, query, results: vec![] });
            }

            // ── Queries ───────────────────────────────────────────────────────
            ClientOperation::QueryDownloadByToken(peer_token, tx) => {
                let download = self.downloads.find_by_peer_token(peer_token).cloned();
                let _ = tx.send(download);
            }
            ClientOperation::QueryDownloads(tx) => {
                let _ = tx.send(self.downloads.get_all());
            }
            ClientOperation::QuerySearchResults(query, tx) => {
                let _ = tx.send(
                    self.searches
                        .values()
                        .find(|s| s.query == query)
                        .map(|s| s.results.clone())
                        .unwrap_or_default(),
                );
            }
        }
    }

    fn peer_connector(&self) -> PeerConnector {
        PeerConnector {
            context: self.context.clone(),
            own_username: self.own_username.clone(),
            op_tx: self.op_tx.clone(),
        }
    }
}

/// Encapsulates the shared context needed to initiate a peer connection.
pub(super) struct PeerConnector {
    context: Arc<ClientContext>,
    own_username: String,
    op_tx: UnboundedSender<ClientOperation>,
}

impl PeerConnector {
    /// Register a P-type (messaging) peer connection.
    pub fn connect_p(&self, peer: Peer, stream: Option<std::net::TcpStream>) {
        let _username = peer.username.clone();
        trace!(
            "[worker] connecting P-type to {}, token {:?}",
            _username, peer.token
        );
        let tokio_stream = stream.and_then(|s| {
            s.set_nonblocking(true).ok();
            tokio::net::TcpStream::from_std(s).ok()
        });
        if let Err(_e) = self.context.peer_registry.register_peer(peer, tokio_stream, None) {
            trace!("Failed to spawn peer actor for {:?}: {:?}", _username, _e);
        }
    }

    /// Initiate an F-type (pierce-firewall) download connection.
    ///
    /// `initiating_token` is the `DownloadToken` for the pending download from this peer
    /// (used as a fallback for pre-token failure reporting). `stream` is `Some` when the
    /// peer is already connected (inbound); `None` when we need to dial out.
    pub fn connect_f(
        &self,
        peer: Peer,
        initiating_token: Option<DownloadToken>,
        stream: Option<std::net::TcpStream>,
    ) {
        trace!(
            "[worker] downloading F-type from: {}, {:?}",
            peer.username, peer.token
        );
        let own_username = self.own_username.clone();
        let op_tx = self.op_tx.clone();
        let download_peer = DownloadPeer::new(
            peer.username,
            peer.host,
            peer.port,
            peer.token.unwrap().0,
            own_username,
        );

        tokio::task::spawn_blocking(move || {
            // Live resolve closure: queries the worker for the download by peer transfer token.
            // Safe to block_on here because spawn_blocking runs on a dedicated thread.
            let op_tx_resolve = op_tx.clone();
            let resolve = move |token: PeerTransferToken| -> Option<Download> {
                let (tx, rx) = tokio::sync::oneshot::channel();
                let _ = op_tx_resolve.send(ClientOperation::QueryDownloadByToken(token, tx));
                tokio::runtime::Handle::current().block_on(rx).ok().flatten()
            };

            match download_peer.download_pierced(resolve, stream) {
                Ok((download, path)) => {
                    trace!("[worker] pierced download complete: {}", path);
                    let _ = op_tx.send(ClientOperation::DownloadCompleted(download.token, Ok(path)));
                }
                Err((maybe_peer_token, e)) => {
                    let our_error = match &e {
                        crate::peer::download_peer::DownloadError::Cancelled => {
                            trace!("[worker] pierced download cancelled");
                            crate::error::SoulseekRs::DownloadCancelled
                        }
                        crate::peer::download_peer::DownloadError::NoProgressTimeout => {
                            trace!("[worker] pierced download timed out");
                            crate::error::SoulseekRs::DownloadTimedOut
                        }
                        _ => {
                            error!("[worker] pierced download failed: {}", e);
                            crate::error::SoulseekRs::InvalidMessage(e.to_string())
                        }
                    };

                    // Try to find our DownloadToken from the peer transfer token.
                    let our_token = if let Some(peer_token) = maybe_peer_token {
                        let (tx, rx) = tokio::sync::oneshot::channel();
                        let _ = op_tx.send(ClientOperation::QueryDownloadByToken(peer_token, tx));
                        tokio::runtime::Handle::current()
                            .block_on(rx)
                            .ok()
                            .flatten()
                            .map(|d| d.token)
                    } else {
                        warn!("[worker] pierce-firewall pre-token failure: {}", e);
                        None
                    }
                    .or(initiating_token);

                    if let Some(token) = our_token {
                        let _ = op_tx.send(ClientOperation::DownloadCompleted(token, Err(our_error)));
                    }
                }
            }
        });
    }
}
