use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::mpsc::UnboundedSender;

use super::download_slot::DownloadSlot;
use crate::client::context::ClientContext;
use crate::client::ClientOperation;
use crate::path::SoulseekPath;
use crate::token::{DownloadToken, PeerTransferToken};
use crate::types::{Download, DownloadStatus, Transfer};
use crate::{error, info, trace};

/// How long to wait for a peer to respond to `QueueUpload` (first response or queue update).
const QUEUE_RESPONSE_TIMEOUT: Duration = Duration::from_secs(180);

/// Owns the download concurrency queue and all download lifecycle logic.
///
/// Extracted from [`ConnectedWorker`] so the worker's `handle_operation` can be a
/// thin dispatcher — each arm delegates to a single method here.
pub struct DownloadManager {
    op_tx: UnboundedSender<ClientOperation>,
    context: Arc<ClientContext>,
    pub logged_in: bool,
    /// Tokens of downloads waiting for a concurrency slot.
    pub pending: VecDeque<DownloadToken>,
    pub max_concurrent: Option<u32>,
    /// One slot per in-flight download; `len()` == active count.
    pub active_slots: HashMap<DownloadToken, DownloadSlot>,
    /// All known downloads (queued or in-flight).
    pub downloads: HashMap<DownloadToken, Download>,
}

impl DownloadManager {
    pub fn new(
        op_tx: UnboundedSender<ClientOperation>,
        context: Arc<ClientContext>,
        max_concurrent: Option<u32>,
        logged_in: bool,
        pending: VecDeque<DownloadToken>,
        downloads: HashMap<DownloadToken, Download>,
    ) -> Self {
        Self {
            op_tx,
            context,
            logged_in,
            pending,
            max_concurrent,
            active_slots: HashMap::new(),
            downloads,
        }
    }

    /// Insert a download and either immediately initiate it or push to the pending queue.
    pub fn enqueue(&mut self, download: Download) {
        let token = download.token;
        // Notify caller immediately that the download was accepted.
        let _ = download.sender.send(DownloadStatus::QueuedLocally);
        self.downloads.insert(token, download);
        if self.logged_in
            && self
                .max_concurrent
                .is_none_or(|max| (self.active_slots.len() as u32) < max)
        {
            self.try_initiate(token);
        } else {
            self.pending.push_back(token);
        }
    }

    /// Send `QueueUpload` to the peer, acquire a concurrency slot, and arm the response timeout.
    pub fn try_initiate(&mut self, token: DownloadToken) {
        let (username, filename) = match self.downloads.get(&token) {
            Some(d) => (d.username.clone(), d.filename.clone()),
            None => return,
        };

        let _ = self.context.peer_registry.queue_upload(&username, filename);
        self.active_slots.insert(token, DownloadSlot);
        if let Some(d) = self.downloads.get_mut(&token) {
            d.status = DownloadStatus::QueuedLocally;
        }

        let op_tx = self.op_tx.clone();
        let handle = tokio::spawn(async move {
            tokio::time::sleep(QUEUE_RESPONSE_TIMEOUT).await;
            let _ = op_tx.send(ClientOperation::DownloadResponseTimeout(token));
        })
        .abort_handle();
        if let Some(d) = self.downloads.get_mut(&token) {
            d.queue_timeout_handle = Some(handle);
        }
    }

    /// Drain the pending queue up to the concurrency limit (called on `LoginSucceeded`).
    pub fn drain_pending_queue(&mut self) {
        loop {
            if self
                .max_concurrent
                .is_some_and(|max| (self.active_slots.len() as u32) >= max)
            {
                break;
            }
            match self.pending.pop_front() {
                Some(token) => self.try_initiate(token),
                None => break,
            }
        }
    }

    /// Dequeue and initiate the next pending download if a slot is available.
    pub fn try_dequeue_next(&mut self) {
        if !self.logged_in {
            return;
        }
        if self
            .max_concurrent
            .is_some_and(|max| (self.active_slots.len() as u32) >= max)
        {
            return;
        }
        if let Some(token) = self.pending.pop_front() {
            self.try_initiate(token);
        }
    }

    /// Download completed (success or failure): update status, free slot, dequeue next.
    pub fn on_completed(
        &mut self,
        token: DownloadToken,
        result: Result<String, crate::error::SoulseekRs>,
    ) {
        let status = match result {
            Ok(ref _path) => {
                info!("Successfully downloaded to {}", _path);
                DownloadStatus::Completed
            }
            Err(crate::error::SoulseekRs::DownloadCancelled) => DownloadStatus::Cancelled,
            Err(crate::error::SoulseekRs::DownloadTimedOut) => DownloadStatus::TimedOut,
            Err(ref _e) => {
                error!("Download failed: {}", _e);
                DownloadStatus::Failed
            }
        };
        if let Some(download) = self.downloads.remove(&token) {
            let _ = download.sender.send(status);
        }
        self.active_slots.remove(&token);
        self.try_dequeue_next();
    }

    /// Fail all downloads from a peer that has disconnected.
    pub fn on_peer_disconnected(&mut self, username: &str) {
        self.fail_matching(username, None);
    }

    /// Fail a specific download that the peer reported as permanently failed.
    pub fn on_upload_failed(&mut self, username: &str, filename: &SoulseekPath) {
        self.fail_matching(username, Some(filename));
    }

    fn fail_matching(&mut self, username: &str, filename: Option<&SoulseekPath>) {
        let failed_tokens: Vec<_> = self
            .downloads
            .values()
            .filter(|d| d.username == username && filename.is_none_or(|f| d.filename == *f))
            .map(|d| {
                let _ = d.sender.send(DownloadStatus::Failed);
                d.token
            })
            .collect();
        let any_failed = !failed_tokens.is_empty();
        for token in failed_tokens {
            self.pending.retain(|t| *t != token);
            if let Some(d) = self.downloads.remove(&token)
                && let Some(h) = d.queue_timeout_handle {
                h.abort();
            }
            self.active_slots.remove(&token);
        }
        if any_failed {
            self.try_dequeue_next();
        }
    }

    /// Cancel a download: remove from pending queue, downloads map, and active slots.
    pub fn on_cancel(&mut self, token: DownloadToken) {
        self.pending.retain(|t| *t != token);
        if let Some(download) = self.downloads.remove(&token) {
            if let Some(h) = download.queue_timeout_handle {
                h.abort();
            }
            let _ = download.sender.send(DownloadStatus::Cancelled);
            if self.active_slots.remove(&token).is_some() {
                self.try_dequeue_next();
            }
        }
    }

    /// Queue-response timeout fired; time out if still `QueuedLocally`.
    pub fn on_timeout(&mut self, token: DownloadToken) {
        let still_waiting = self
            .downloads
            .get(&token)
            .is_some_and(|d| matches!(d.status, DownloadStatus::QueuedLocally));
        if still_waiting {
            if let Some(download) = self.downloads.remove(&token) {
                let _ = download.sender.send(DownloadStatus::TimedOut);
            }
            if self.active_slots.remove(&token).is_some() {
                self.try_dequeue_next();
            }
        }
    }

    /// Peer's `TransferRequest` received: record peer token, abort queue timeout, → `QueuedRemotely`.
    pub fn on_transfer_request(&mut self, transfer: &Transfer, username: &str) {
        if let Some(download) = self
            .downloads
            .values_mut()
            .find(|d| d.username == username && d.filename == transfer.filename)
        {
            trace!(
                "[dm] TransferRequest: {} peer_token={} size={}",
                download.token,
                transfer.token,
                transfer.size
            );
            download.peer_token = Some(transfer.token);
            download.size = transfer.size;
            if let Some(h) = download.queue_timeout_handle.take() {
                h.abort();
            }
            download.status = DownloadStatus::QueuedRemotely { place: None };
            let _ = download.sender.send(DownloadStatus::QueuedRemotely { place: None });
        }
    }

    /// `PlaceInQueueResponse` received: update queue position, reset 3-minute timeout.
    pub fn on_queue_position(&mut self, username: &str, filename: &SoulseekPath, place: u32) {
        if let Some(download) = self
            .downloads
            .values_mut()
            .find(|d| d.username == username && d.filename == *filename)
        {
            if let Some(h) = download.queue_timeout_handle.take() {
                h.abort();
            }
            download.status = DownloadStatus::QueuedRemotely { place: Some(place) };
            let _ = download.sender.send(DownloadStatus::QueuedRemotely { place: Some(place) });
            let op_tx = self.op_tx.clone();
            let token = download.token;
            let handle = tokio::spawn(async move {
                tokio::time::sleep(QUEUE_RESPONSE_TIMEOUT).await;
                let _ = op_tx.send(ClientOperation::DownloadResponseTimeout(token));
            })
            .abort_handle();
            download.queue_timeout_handle = Some(handle);
        }
    }

    /// Peer sent `TransferResponse(allowed=false)`: stay queued remotely, reset timeout.
    pub fn on_transfer_rejected(&mut self, peer_token: PeerTransferToken) {
        if let Some(download) = self
            .downloads
            .values_mut()
            .find(|d| d.peer_token == Some(peer_token))
        {
            if let Some(h) = download.queue_timeout_handle.take() {
                h.abort();
            }
            download.status = DownloadStatus::QueuedRemotely { place: None };
            let _ = download.sender.send(DownloadStatus::QueuedRemotely { place: None });
            let op_tx = self.op_tx.clone();
            let token = download.token;
            let handle = tokio::spawn(async move {
                tokio::time::sleep(QUEUE_RESPONSE_TIMEOUT).await;
                let _ = op_tx.send(ClientOperation::DownloadResponseTimeout(token));
            })
            .abort_handle();
            download.queue_timeout_handle = Some(handle);
        }
    }

    /// Find a download by peer transfer token (for `DownloadFromPeer` / listener lookups).
    pub fn find_by_peer_token(&self, token: PeerTransferToken) -> Option<&Download> {
        self.downloads.values().find(|d| d.peer_token == Some(token))
    }

    /// Find the `DownloadToken` for an active (non-finished) download from the given peer username.
    /// Used as the pre-token fallback in `connect_f` failure reporting.
    pub fn find_initiating_token(&self, username: &str) -> Option<DownloadToken> {
        self.downloads
            .values()
            .find(|d| d.username == username && !d.is_finished())
            .map(|d| d.token)
    }

    pub fn on_server_disconnected(&mut self) {
        self.logged_in = false;
    }

    pub fn on_login_succeeded(&mut self) {
        self.logged_in = true;
        self.drain_pending_queue();
    }

    pub fn get_all(&self) -> Vec<Download> {
        self.downloads.values().cloned().collect()
    }
}
