use crate::actor::server_actor::{ServerActor, ServerActorConfig, ServerCommand};
use crate::path::SoulseekPath;
use crate::search_rate_limiter::SlidingRateLimiter;
use crate::token::{DownloadToken, SearchToken};
use crate::types::DownloadStatus;
use crate::{
    actor::peer_registry::PeerRegistry,
    error::{Result, SoulseekRs},
    peer::listen::Listen,
    types::{Download, SearchResult},
    utils::md5,
};
use crate::{error, info, trace};
use std::collections::HashMap;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use tokio::sync::oneshot;

mod connected_worker;
mod context;
mod download_handle;
mod download_manager;
mod download_slot;
mod inner;
pub(super) mod operation;
mod settings;

use connected_worker::ConnectedWorker;
use download_manager::DownloadManager;
pub use context::ClientContext;
pub use download_handle::DownloadHandle;
pub use inner::{ActiveConnection, ClientInner, ClientState};
pub use operation::ClientOperation;
pub use settings::*;

#[derive(Clone)]
pub struct Client {
    settings: ClientSettings,
    /// Single lock for all mutable state.
    inner: Arc<Mutex<ClientInner>>,
    /// The search rate limiter.
    /// This is used to limit the number of searches that can be performed concurrently.
    /// It is used to prevent abuse, and being banned.
    search_limiter: Option<SlidingRateLimiter>,
}

impl Client {
    async fn wait_for_search_rate_limit(&self) -> Result<()> {
        if let Some(wait) = self
            .search_limiter
            .as_ref()
            .map(|lim| lim.clone().acquire())
        {
            wait.await
        }

        Ok(())
    }

    pub fn new(
        username: impl Into<PlainTextUnencrypted>,
        password: impl Into<PlainTextUnencrypted>,
    ) -> Self {
        Self::with_settings(ClientSettings::new(username, password))
    }

    pub fn with_settings(settings: ClientSettings) -> Self {
        let search_limiter = settings
            .search_rate_limit_settings
            .as_ref()
            .map(|s| SlidingRateLimiter::new(s.searches, s.per_period));

        Self {
            settings,
            inner: Arc::new(Mutex::new(ClientInner {
                state: ClientState::Disconnected,
                active: None,
                pending_downloads: VecDeque::new(),
            })),
            search_limiter,
        }
    }

    /// Connect to the Soulseek server and login. Blocks until login succeeds or fails.
    pub async fn connect(&self) -> Result<()> {
        trace!("Connecting to soulseek");
        {
            let guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
            if matches!(guard.state, ClientState::Connected) {
                return Ok(());
            }
        }

        self.inner.lock().unwrap_or_else(|e| e.into_inner()).state = ClientState::Connecting;

        let username = self.settings.username.0.clone();
        let password = self.settings.password.0.clone();
        let max_concurrent = self
            .settings
            .download_rate_limit_settings
            .as_ref()
            .map(|s| s.concurrent_downloads);

        // Create actor_system upfront — no lock needed later to retrieve it.
        let actor_system = Arc::new(crate::actor::ActorSystem::new());
        let cancellation_token = actor_system.cancellation_token().clone();

        let (op_tx, op_rx) = mpsc::unbounded_channel::<ClientOperation>();

        // Build fully-initialized context before spawning anything.
        let context = {
            let peer_registry = PeerRegistry::new(
                actor_system.clone(),
                op_tx.clone(),
                username.clone(),
                self.settings.shared_folders,
                self.settings.shared_files,
            );
            Arc::new(ClientContext::new(peer_registry))
        };

        // Drain any pre-connect pending downloads into the worker's download manager.
        let (pre_pending_tokens, pre_downloads) = {
            let mut guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
            let mut downloads: HashMap<DownloadToken, Download> = HashMap::new();
            let tokens: VecDeque<DownloadToken> = guard
                .pending_downloads
                .drain(..)
                .map(|d| {
                    let token = d.token;
                    downloads.insert(token, d);
                    token
                })
                .collect();
            (tokens, downloads)
        };

        // Spawn ServerActor first so we have its handle for the worker.
        let server_actor = ServerActor::new(
            self.settings.server_address.clone(),
            op_tx.clone(),
            ServerActorConfig {
                listen_port: self.settings.listen_port,
                enable_listen: self.settings.enable_listen,
                shared_folders: self.settings.shared_folders,
                shared_files: self.settings.shared_files,
                tcp_keepalive: self.settings.tcp_keepalive_settings.clone(),
                reconnect_settings: self.settings.reconnect_settings.clone(),
            },
        );
        let server_handle = actor_system.spawn(server_actor);

        // Spawn ConnectedWorker — receives server_handle for sending commands.
        let worker = ConnectedWorker {
            own_username: username.clone(),
            op_tx: op_tx.clone(),
            op_rx,
            inner: self.inner.clone(),
            context: context.clone(),
            cancellation_token: cancellation_token.clone(),
            server_handle: server_handle.clone(),
            downloads: DownloadManager::new(
                op_tx.clone(),
                server_handle.clone(),
                context.clone(),
                max_concurrent,
                false,
                pre_pending_tokens,
                pre_downloads,
            ),
            searches: HashMap::new(),
        };
        tokio::spawn(async move { worker.run().await });

        // Store active connection (after server handle exists).
        {
            let mut guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
            guard.active = Some(ActiveConnection {
                server_handle: server_handle.clone(),
                op_tx: op_tx.clone(),
                actor_system: actor_system.clone(),
            });
        }

        // Spawn listener task
        if self.settings.enable_listen {
            let listen_port = self.settings.listen_port;
            let own_username = username.clone();
            let token = cancellation_token.clone();

            tokio::spawn(async move {
                tokio::select! {
                    _ = token.cancelled() => {
                        trace!("[listener] Shutdown signal received");
                    }
                    result = Listen::start(listen_port, op_tx, context, own_username) => {
                        match result {
                            Ok(_) => info!("[listener] Listener started successfully"),
                            Err(_e) => error!("[listener] Failed to start listener: {}", _e),
                        }
                    }
                }
            });
        }

        // Send Login — queued by ServerActor until TCP is ready.
        let (login_tx, login_rx) = oneshot::channel();
        let _ = server_handle.send(ServerCommand::Login {
            username: username.clone(),
            password: password.clone(),
            response: login_tx,
        });

        // Await login result. ClientState::Connected is set by state_monitor on LoginSucceeded.
        match login_rx.await {
            Ok(Ok(true)) => Ok(()),
            Ok(Err(e)) => Err(e),
            Ok(Ok(false)) => Err(SoulseekRs::AuthenticationFailed),
            Err(_) => Err(SoulseekRs::Timeout),
        }
    }

    pub async fn search_with_cancel(
        &self,
        query: &str,
        timeout: Duration,
        cancel_flag: Option<Arc<AtomicBool>>,
    ) -> Result<Vec<SearchResult>> {
        info!("Searching for {}", query);

        // Get connection handles
        let (server_handle, op_tx) = {
            self.wait_for_search_rate_limit().await?;
            let guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
            match guard.active.as_ref() {
                Some(active) => (active.server_handle.clone(), active.op_tx.clone()),
                None => return Err(SoulseekRs::NotConnected),
            }
        };

        let hash = md5::md5(query);
        let token = SearchToken(u32::from_str_radix(&hash[0..5], 16)?);

        // Register search in worker and send FileSearch to server
        let _ = op_tx.send(ClientOperation::InitiateSearch(token, query.to_string()));
        let _ = server_handle.send(ServerCommand::FileSearch {
            token,
            query: query.to_string(),
        });

        // Poll until timeout or cancel
        let start = Instant::now();
        loop {
            tokio::time::sleep(Duration::from_millis(100)).await;

            if let Some(ref flag) = cancel_flag
                && flag.load(Ordering::Relaxed)
            {
                info!("Search cancelled by user");
                break;
            }

            if start.elapsed() >= timeout {
                break;
            }
        }

        // Query results from worker
        let (tx, rx) = oneshot::channel();
        let _ = op_tx.send(ClientOperation::QuerySearchResults(query.to_string(), tx));
        rx.await.map_err(|_| SoulseekRs::NotConnected)
    }

    pub async fn search(&self, query: &str, timeout: Duration) -> Result<Vec<SearchResult>> {
        self.search_with_cancel(query, timeout, None).await
    }

    pub fn download(
        &self,
        filename: impl Into<SoulseekPath>,
        username: String,
        size: u64,
        download_directory: String,
        progress_timeout: Option<Duration>,
        recv_timeout: Option<Duration>,
    ) -> Result<(Download, DownloadHandle)> {
        let filename: SoulseekPath = filename.into();
        info!("[client] Downloading {} from {}", filename, username);

        let hash = md5::md5(filename.as_str());
        let token = DownloadToken(u32::from_str_radix(&hash[0..5], 16)?);

        let (download_sender, download_receiver) = mpsc::unbounded_channel::<DownloadStatus>();
        let cancel = Arc::new(AtomicBool::new(false));

        let download = Download {
            username: username.clone(),
            filename: filename.clone(),
            token,
            peer_token: None,
            size,
            download_directory,
            status: DownloadStatus::QueuedLocally,
            sender: download_sender,
            cancel: cancel.clone(),
            progress_timeout,
            queue_timeout_handle: None,
        };

        let mut guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());

        let op_tx_opt = guard.active.as_ref().map(|a| a.op_tx.clone());
        let handle = DownloadHandle::new(
            download_receiver,
            cancel,
            progress_timeout,
            recv_timeout,
            op_tx_opt,
            token,
        );

        if let Some(ref active) = guard.active {
            // Active connection — worker handles insertion and routing.
            let _ = active.op_tx.send(ClientOperation::RequestDownload(download.clone()));
        } else {
            // No active connection yet — buffer until connect().
            guard.pending_downloads.push_back(download.clone());
        }

        Ok((download, handle))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_client_context_construction() {
        use crate::actor::ActorSystem;
        let actor_system = Arc::new(ActorSystem::new());
        let (op_tx, _) = mpsc::unbounded_channel();
        let registry = PeerRegistry::new(actor_system, op_tx, "test".to_string(), 1, 499);
        let context = ClientContext::new(registry);
        // Just verify it constructs without panic
        drop(context);
    }

    #[test]
    fn test_client_settings_default_compiles() {
        let settings = ClientSettings::default();
        assert_eq!(settings.listen_port, 2234);
        assert!(settings.search_rate_limit_settings.is_some());
        assert!(settings.download_rate_limit_settings.is_some());
    }
}
