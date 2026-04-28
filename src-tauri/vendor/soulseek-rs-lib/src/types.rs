use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use tokio::sync::mpsc::UnboundedSender;
use tokio::task::AbortHandle;

use crate::token::{DownloadToken, PeerTransferToken, SearchToken};
use crate::{error::Result, message::Message, path::SoulseekPath, utils::zlib::deflate};

#[derive(Debug, Clone, Default)]
pub struct FileAttributes {
    pub bitrate: Option<u32>,     // kbps (code 0)
    pub duration: Option<u32>,    // seconds (code 1)
    pub vbr: Option<bool>,        // variable bit rate flag (code 2)
    pub sample_rate: Option<u32>, // Hz (code 4)
    pub bit_depth: Option<u32>,   // bits (code 5)
}

impl FileAttributes {
    fn from_map(map: HashMap<u32, u32>) -> Self {
        Self {
            bitrate: map.get(&0).copied(),
            duration: map.get(&1).copied(),
            vbr: map.get(&2).map(|&v| v != 0),
            sample_rate: map.get(&4).copied(),
            bit_depth: map.get(&5).copied(),
        }
    }
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct File {
    pub username: String,
    pub name: SoulseekPath,
    pub size: u64,
    pub attributes: FileAttributes,
}
pub struct UploadFailed {
    pub filename: SoulseekPath,
}
impl UploadFailed {
    pub fn new_from_message(message: &mut Message) -> Self {
        let filename = SoulseekPath::from_wire(message.read_string());

        Self { filename }
    }
}
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct SearchResult {
    pub token: SearchToken,
    pub files: Vec<File>,
    pub slots: u8,
    pub speed: u32,
    pub username: String,
}

#[derive(Debug, Clone)]
pub struct Search {
    pub token:   SearchToken,
    pub query:   String,
    pub results: Vec<SearchResult>,
}

impl SearchResult {
    pub fn new_from_message(message: &mut Message) -> Result<Self> {
        let pointer = message.get_pointer();
        let size = message.get_size();
        let data: Vec<u8> = message.get_slice(pointer, size);
        let deflated = deflate(&data)?;
        let mut message = Message::new_with_data(deflated);

        let username = message.read_string();
        let token = SearchToken(message.read_int32());
        let n_files = message.read_int32();
        let mut files: Vec<File> = Vec::new();
        for _ in 0..n_files {
            message.read_int8();
            let name = message.read_string();
            let size = message.read_int64();
            message.read_string();
            let n_attribs = message.read_int32();
            let mut attribs: HashMap<u32, u32> = HashMap::new();

            for _ in 0..n_attribs {
                attribs.insert(message.read_int32(), message.read_int32());
            }
            files.push(File {
                username: username.clone(),
                name: SoulseekPath::from_wire(name),
                size,
                attributes: FileAttributes::from_map(attribs),
            });
        }
        let slots = message.read_int8();
        let speed = message.read_int32();

        Ok(Self {
            token,
            files,
            slots,
            speed,
            username,
        })
    }
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct Transfer {
    pub direction: u32,
    pub token: PeerTransferToken,
    pub filename: SoulseekPath,
    pub size: u64,
}

/// Represents an active or completed download tracked by the worker.
///
/// Created from `Client::download()` and sent to the `ConnectedWorker` via
/// `ClientOperation::RequestDownload`. Lives in `DownloadManager::downloads`
/// for the lifetime of the transfer.
#[derive(Debug, Clone)]
pub struct Download {
    /// The Soulseek username of the peer providing the file.
    pub username: String,
    /// The full Soulseek path of the file being downloaded.
    pub filename: SoulseekPath,
    /// Unique token identifying this download. Initially assigned by us; may be
    /// remapped when the peer sends a `TransferRequest` with a different token.
    pub token: DownloadToken,
    /// The peer's wire token, set when `TransferRequest` is received.
    /// `None` until the uploader sends `TransferRequest`.
    pub peer_token: Option<PeerTransferToken>,
    /// Expected file size in bytes as reported by the peer.
    pub size: u64,
    /// Local filesystem directory where the downloaded file will be written.
    pub download_directory: String,
    /// Current lifecycle status; updated by the worker on each state transition.
    pub status: DownloadStatus,
    /// Sender half of the progress channel. The worker sends [`DownloadStatus`]
    /// updates here; the user receives them via [`DownloadHandle`].
    ///
    /// [`DownloadHandle`]: crate::client::download_handle::DownloadHandle
    pub sender: UnboundedSender<DownloadStatus>,
    /// Shared cancel flag — set to `true` by [`DownloadHandle::cancel`] to request
    /// cancellation. Checked periodically by the download task.
    ///
    /// [`DownloadHandle::cancel`]: crate::client::download_handle::DownloadHandle::cancel
    pub cancel: Arc<AtomicBool>,
    /// If set, the download is cancelled when no progress update arrives within
    /// this duration. Used to detect stalled transfers.
    pub progress_timeout: Option<Duration>,
    /// Abort handle for the queue-response timeout task.
    /// Aborted when the download transitions out of `QueuedLocally`
    /// (i.e. when `TransferRequest` or a queue-position update is received).
    pub queue_timeout_handle: Option<AbortHandle>,
}

impl Download {
    pub fn is_finished(&self) -> bool {
        matches!(
            self.status,
            DownloadStatus::Completed
                | DownloadStatus::Failed
                | DownloadStatus::TimedOut
                | DownloadStatus::Cancelled
        )
    }

    pub fn bytes_downloaded(&self) -> u64 {
        match &self.status {
            DownloadStatus::InProgress {
                bytes_downloaded, ..
            } => *bytes_downloaded,
            DownloadStatus::Completed => self.size,
            _ => 0,
        }
    }

    pub fn speed_bytes_per_sec(&self) -> f64 {
        match &self.status {
            DownloadStatus::InProgress {
                speed_bytes_per_sec,
                ..
            } => *speed_bytes_per_sec,
            _ => 0.0,
        }
    }
}

#[derive(Debug, Clone)]
pub enum DownloadStatus {
    /// Waiting for a local concurrency slot; `QueueUpload` not yet sent to peer.
    QueuedLocally,
    /// `QueueUpload` sent; peer acknowledged and we are in their upload queue.
    /// `place` is `Some` when a `PlaceInQueueResponse` has been received.
    QueuedRemotely { place: Option<u32> },
    InProgress {
        bytes_downloaded: u64,
        total_bytes: u64,
        speed_bytes_per_sec: f64,
    },
    Completed,
    Failed,
    TimedOut,
    Cancelled,
}
impl Transfer {
    pub fn new_from_message(message: &mut Message) -> Self {
        let direction = message.read_int32();
        let token = PeerTransferToken(message.read_int32());
        let filename = SoulseekPath::from_wire(message.read_string());
        let size = message.read_int64();

        Self {
            direction,
            token,
            filename,
            size,
        }
    }
}
