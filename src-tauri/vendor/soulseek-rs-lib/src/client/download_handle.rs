use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::mpsc::UnboundedReceiver;
use tokio::sync::mpsc::UnboundedSender;
use tokio::time::sleep;

use crate::client::ClientOperation;
use crate::token::DownloadToken;
use crate::types::DownloadStatus;

const DEFAULT_RECV_TIMEOUT: Duration = Duration::from_secs(120);

/// Handle returned by [`Client::download`] for receiving progress and cancelling a download.
///
/// Dropping this handle automatically cancels the download.
pub struct DownloadHandle {
    receiver: UnboundedReceiver<DownloadStatus>,
    cancel: Arc<AtomicBool>,
    /// Passed to the download loop — cancels if no bytes arrive within this window.
    #[allow(dead_code)]
    progress_timeout: Option<Duration>,
    /// Used only by [`recv`](Self::recv) — cancels the wait if no status update arrives.
    recv_timeout: Option<Duration>,
    /// Sender to the worker for cancellation cleanup (None if disconnected at download time).
    op_tx: Option<UnboundedSender<ClientOperation>>,
    token: DownloadToken,
}

impl DownloadHandle {
    pub(super) fn new(
        receiver: UnboundedReceiver<DownloadStatus>,
        cancel: Arc<AtomicBool>,
        progress_timeout: Option<Duration>,
        recv_timeout: Option<Duration>,
        op_tx: Option<UnboundedSender<ClientOperation>>,
        token: DownloadToken,
    ) -> Self {
        Self {
            receiver,
            cancel,
            progress_timeout,
            recv_timeout,
            op_tx,
            token,
        }
    }

    /// Signal the download to cancel. The next [`DownloadStatus::Cancelled`] update will arrive
    /// shortly after via [`recv`](Self::recv).
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    /// Receive the next status update, or `None` if the channel is closed.
    ///
    /// Times out after `recv_timeout` (default 10 minutes) and returns
    /// `Some(DownloadStatus::Cancelled)` if no update arrives in time.
    pub async fn recv(&mut self) -> Option<DownloadStatus> {
        let recv_timeout = self.recv_timeout.unwrap_or(DEFAULT_RECV_TIMEOUT);
        tokio::select! {
            result = self.receiver.recv() => {
                result
            }
            _ = sleep(recv_timeout) => {
                self.cancel();
                Some(DownloadStatus::Cancelled)
            }
        }
    }

    /// Non-blocking receive — returns `None` if no update is available yet.
    pub fn try_recv(&mut self) -> Option<DownloadStatus> {
        self.receiver.try_recv().ok()
    }
}

impl Drop for DownloadHandle {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
        if let Some(ref tx) = self.op_tx {
            let _ = tx.send(ClientOperation::CancelDownload(self.token));
        }
    }
}
