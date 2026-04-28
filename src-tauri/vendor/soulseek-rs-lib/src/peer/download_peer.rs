use std::env;
use std::fs;
use std::io::{self, Read, Write};
use std::net::TcpStream;
use std::net::ToSocketAddrs;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use tokio::sync::mpsc::UnboundedSender;

use crate::client::ClientOperation;
use crate::error::SoulseekRs;
use crate::message::server::MessageFactory;
use crate::token::PeerTransferToken;
use crate::{error, trace};
use crate::types::{Download, DownloadStatus};

const START_DOWNLOAD: [u8; 8] = [0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00];
const READ_BUFFER_SIZE: usize = 8192;
const PROGRESS_UPDATE_CHUNKS: usize = 15; // ~120KB (15 * 8192 bytes)
/// How often the read loop wakes up to check cancel/timeout flags when the peer is silent.
const READ_CHECK_INTERVAL: Duration = Duration::from_secs(1);
/// Hard stall timeout: give up if no bytes arrive within this window.
const STALL_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug)]
pub enum DownloadError {
    ConnectionFailed(io::Error),
    InvalidAddress(String),
    HandshakeFailed(io::Error),
    StreamReadError(io::Error),
    StreamWriteError(io::Error),
    TokenNotFound(PeerTransferToken),
    DownloadInfoMissing,
    FileWriteError(io::Error),
    PathResolutionError(String),
    InvalidTokenBytes,
    /// Download was cancelled via [`Download::cancel`].
    Cancelled,
    /// No progress received within the configured timeout duration.
    NoProgressTimeout,
}

impl From<DownloadError> for SoulseekRs {
    fn from(e: DownloadError) -> Self {
        match e {
            DownloadError::Cancelled => SoulseekRs::DownloadCancelled,
            DownloadError::NoProgressTimeout => SoulseekRs::DownloadTimedOut,
            other => SoulseekRs::InvalidMessage(other.to_string()),
        }
    }
}

/// Spawns a blocking task that runs a direct download and reports the result
/// back to the worker via [`ClientOperation::DownloadCompleted`].
///
/// Use this for all `download_direct` paths (outbound and inbound) so error
/// mapping and the send pattern stay in one place.
pub fn spawn_direct_download(
    download: Download,
    host: String,
    port: u32,
    peer_token: u32,
    own_username: String,
    stream: Option<TcpStream>,
    op_tx: UnboundedSender<ClientOperation>,
) {
    let token = download.token;
    let peer = DownloadPeer::new(download.username.clone(), host.clone(), port, peer_token, own_username);
    tokio::task::spawn_blocking(move || {
        let result = peer
            .download_direct(download, stream)
            .map(|(_, path)| path)
            .map_err(|e| {
                if !matches!(e, DownloadError::Cancelled | DownloadError::NoProgressTimeout) {
                    error!("Failed to download from {}:{} (token: {}): {}", host, port, token, e);
                }
                SoulseekRs::from(e)
            });
        let _ = op_tx.send(ClientOperation::DownloadCompleted(token, result));
    });
}

impl std::fmt::Display for DownloadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ConnectionFailed(e) => write!(f, "Connection failed: {}", e),
            Self::InvalidAddress(addr) => write!(f, "Invalid address: {}", addr),
            Self::HandshakeFailed(e) => write!(f, "Handshake failed: {}", e),
            Self::StreamReadError(e) => write!(f, "Stream read error: {}", e),
            Self::StreamWriteError(e) => write!(f, "Stream write error: {}", e),
            Self::TokenNotFound(token) => write!(f, "Token not found: {}", token),
            Self::DownloadInfoMissing => write!(f, "Download info missing"),
            Self::FileWriteError(e) => write!(f, "File write error: {}", e),
            Self::PathResolutionError(msg) => write!(f, "Path resolution error: {}", msg),
            Self::InvalidTokenBytes => write!(f, "Invalid token bytes received"),
            Self::Cancelled => write!(f, "Download cancelled"),
            Self::NoProgressTimeout => write!(f, "Download timed out (no progress)"),
        }
    }
}

impl std::error::Error for DownloadError {}

impl From<io::Error> for DownloadError {
    fn from(error: io::Error) -> Self {
        Self::StreamReadError(error)
    }
}

struct FileManager;

impl FileManager {
    fn expand_path(path: &str) -> PathBuf {
        if let Some(stripped) = path.strip_prefix('~') {
            if let Ok(home) = env::var("HOME") {
                PathBuf::from(home).join(stripped.trim_start_matches('/'))
            } else {
                PathBuf::from(path)
            }
        } else {
            PathBuf::from(path)
        }
    }
}

pub struct DownloadPeer {
    username: String,
    host: String,
    port: u32,
    #[allow(dead_code)]
    own_username: String,
    /// Token sent in the pierce-firewall handshake (pierce path only).
    token: u32,
}

impl DownloadPeer {
    #[must_use]
    pub fn new(
        username: String,
        host: String,
        port: u32,
        token: u32,
        own_username: String,
    ) -> Self {
        Self {
            username,
            host,
            port,
            own_username,
            token,
        }
    }

    fn establish_connection(&self) -> Result<TcpStream, DownloadError> {
        let socket_address = format!("{}:{}", self.host, self.port)
            .to_socket_addrs()
            .map_err(DownloadError::ConnectionFailed)?
            .next()
            .ok_or_else(|| DownloadError::InvalidAddress(format!("{}:{}", self.host, self.port)))?;

        let stream = TcpStream::connect_timeout(&socket_address, Duration::from_secs(20))
            .map_err(DownloadError::ConnectionFailed)?;

        // Use a short read timeout so the read loop can check cancel/progress-timeout flags
        // between poll intervals rather than blocking for 30 s on a stalled peer.
        stream
            .set_read_timeout(Some(READ_CHECK_INTERVAL))
            .map_err(DownloadError::ConnectionFailed)?;
        stream
            .set_write_timeout(Some(Duration::from_secs(5)))
            .map_err(DownloadError::ConnectionFailed)?;
        stream
            .set_nodelay(true)
            .map_err(DownloadError::ConnectionFailed)?;

        Ok(stream)
    }

    fn open_output_file(path: &str) -> Result<io::BufWriter<fs::File>, DownloadError> {
        if let Some(parent) = Path::new(path).parent() {
            fs::create_dir_all(parent).map_err(DownloadError::FileWriteError)?;
        }
        let f = fs::File::create(path).map_err(DownloadError::FileWriteError)?;
        Ok(io::BufWriter::new(f))
    }

    fn resolve_download_path(download: &Download) -> Result<String, DownloadError> {
        let download_directory = &download.download_directory;
        let mut expanded_path = FileManager::expand_path(download_directory);

        if !expanded_path.is_dir() {
            expanded_path = expanded_path
                .parent()
                .ok_or_else(|| {
                    DownloadError::PathResolutionError(format!(
                        "Cannot resolve parent directory for: {}",
                        expanded_path.display()
                    ))
                })?
                .to_path_buf();
        }

        let final_path = expanded_path.join(download.filename.filename());

        final_path
            .to_str()
            .ok_or_else(|| {
                DownloadError::PathResolutionError(format!(
                    "Path contains invalid UTF-8: {}",
                    final_path.display()
                ))
            })
            .map(String::from)
    }

    /// Download a file where the download info is known upfront (direct connection).
    ///
    /// Sends `START_DOWNLOAD`, then streams data to disk. Progress is reported
    /// via `download.sender` — no shared context lock is held during I/O.
    pub fn download_direct(
        self,
        download: Download,
        stream: Option<TcpStream>,
    ) -> Result<(Download, String), DownloadError> {
        trace!(
            "[download_peer:{}] download_direct, stream present: {}",
            self.username,
            stream.is_some()
        );

        let mut stream = match stream {
            Some(s) => {
                s.set_nonblocking(false).map_err(DownloadError::ConnectionFailed)?;
                s.set_read_timeout(Some(READ_CHECK_INTERVAL)).map_err(DownloadError::ConnectionFailed)?;
                s.set_write_timeout(Some(Duration::from_secs(5))).map_err(DownloadError::ConnectionFailed)?;
                s.set_nodelay(true).map_err(DownloadError::ConnectionFailed)?;
                s
            }
            None => self.establish_connection()?,
        };

        let path = Self::resolve_download_path(&download)?;
        let mut writer = Self::open_output_file(&path)?;

        stream
            .write_all(&START_DOWNLOAD)
            .map_err(DownloadError::StreamWriteError)?;

        trace!("[download_peer:{}] sent START_DOWNLOAD", self.username);

        let _total_bytes = Self::read_stream(
            &self.username,
            &mut stream,
            &mut writer,
            &download,
        )?;

        writer.flush().map_err(DownloadError::FileWriteError)?;

        trace!(
            "[download_peer:{}] download_direct complete: {} bytes → {}",
            self.username, _total_bytes, path
        );

        Ok((download, path))
    }

    /// Download a file where the download info is resolved from the first bytes of the stream
    /// (pierce-firewall path).
    ///
    /// Sends a pierce-firewall handshake, reads a 4-byte token from the peer,
    /// calls `resolve_download` to look up the corresponding `Download`,
    /// then streams data to disk.
    ///
    /// On error, returns `(Option<PeerTransferToken>, DownloadError)` where the token is `Some` if it
    /// was resolved before the failure, or `None` if the failure occurred during the handshake.
    pub fn download_pierced(
        self,
        resolve_download: impl Fn(PeerTransferToken) -> Option<Download>,
        stream: Option<TcpStream>,
    ) -> Result<(Download, String), (Option<PeerTransferToken>, DownloadError)> {
        trace!(
            "[download_peer:{}] download_pierced, stream present: {}",
            self.username,
            stream.is_some()
        );

        let mut stream = match stream {
            Some(s) => {
                s.set_nonblocking(false).map_err(|e| (None, DownloadError::ConnectionFailed(e)))?;
                s.set_read_timeout(Some(READ_CHECK_INTERVAL)).map_err(|e| (None, DownloadError::ConnectionFailed(e)))?;
                s.set_write_timeout(Some(Duration::from_secs(5))).map_err(|e| (None, DownloadError::ConnectionFailed(e)))?;
                s.set_nodelay(true).map_err(|e| (None, DownloadError::ConnectionFailed(e)))?;
                s
            }
            None => self.establish_connection().map_err(|e| (None, e))?,
        };

        // Send pierce-firewall message so the peer can identify this connection.
        let message = MessageFactory::build_pierce_firewall_message(self.token);
        stream
            .write_all(&message.get_buffer())
            .map_err(|e| (None, DownloadError::HandshakeFailed(e)))?;
        stream.flush().map_err(|e| (None, DownloadError::HandshakeFailed(e)))?;

        trace!(
            "[download_peer:{}] sent pierce firewall token: {}",
            self.username, self.token
        );

        // Read the first chunk — first 4 bytes are the download token.
        // On macOS an immediately readable socket can still transiently return WouldBlock
        // during the pierced-handshake phase, so we retry until we have the token bytes.
        let mut first_buf = [0u8; READ_BUFFER_SIZE];
        let mut first_read = 0usize;
        let mut last_data_time = Instant::now();

        while first_read < 4 {
            match stream.read(&mut first_buf[first_read..]) {
                Ok(0) => {
                    return Err((
                        None,
                        DownloadError::StreamReadError(io::Error::new(
                            io::ErrorKind::UnexpectedEof,
                            "peer closed connection before sending download token",
                        )),
                    ));
                }
                Ok(bytes_read) => {
                    first_read += bytes_read;
                    last_data_time = Instant::now();
                }
                Err(e) if e.kind() == io::ErrorKind::WouldBlock
                    || e.kind() == io::ErrorKind::TimedOut =>
                {
                    if last_data_time.elapsed() >= STALL_TIMEOUT {
                        return Err((
                            None,
                            DownloadError::StreamReadError(io::Error::new(
                                io::ErrorKind::TimedOut,
                                "peer did not send download token",
                            )),
                        ));
                    }
                }
                Err(e) => return Err((None, DownloadError::StreamReadError(e))),
            }
        }

        if first_read < 4 {
            return Err((None, DownloadError::InvalidTokenBytes));
        }

        let token = PeerTransferToken(u32::from_le_bytes(
            first_buf[..4]
                .try_into()
                .map_err(|_| (None, DownloadError::InvalidTokenBytes))?,
        ));

        trace!(
            "[download_peer:{}] resolved download token: {}",
            self.username, token
        );

        // Token is now known — all subsequent errors carry Some(token).
        let download = resolve_download(token)
            .ok_or((Some(token), DownloadError::TokenNotFound(token)))?;

        stream
            .write_all(&START_DOWNLOAD)
            .map_err(|e| (Some(token), DownloadError::StreamWriteError(e)))?;

        let path =
            Self::resolve_download_path(&download).map_err(|e| (Some(token), e))?;
        let mut writer =
            Self::open_output_file(&path).map_err(|e| (Some(token), e))?;

        // Any bytes after the 4-byte token in the first chunk are the start of file data.
        let mut total_bytes: usize = 0;
        if first_read > 4 {
            let initial_data = &first_buf[4..first_read];
            writer
                .write_all(initial_data)
                .map_err(|e| (Some(token), DownloadError::FileWriteError(e)))?;
            total_bytes += initial_data.len();

            // Send progress if the initial chunk is already large enough.
            if total_bytes >= PROGRESS_UPDATE_CHUNKS * READ_BUFFER_SIZE {
                let _ = download.sender.send(DownloadStatus::InProgress {
                    bytes_downloaded: total_bytes as u64,
                    total_bytes: download.size,
                    speed_bytes_per_sec: 0.0,
                });
            }
        }

        let _final_bytes = Self::read_stream_with_offset(
            &self.username,
            &mut stream,
            &mut writer,
            &download,
            total_bytes,
        )
        .map_err(|e| (Some(token), e))?;

        writer
            .flush()
            .map_err(|e| (Some(token), DownloadError::FileWriteError(e)))?;

        trace!(
            "[download_peer:{}] download_pierced complete: {} bytes → {}",
            self.username, total_bytes + _final_bytes, path
        );

        Ok((download, path))
    }

    /// Inner loop: read data from `stream`, write to `writer`, send progress via `download.sender`.
    fn read_stream(
        username: &str,
        stream: &mut TcpStream,
        writer: &mut io::BufWriter<fs::File>,
        download: &Download,
    ) -> Result<usize, DownloadError> {
        Self::read_stream_with_offset(username, stream, writer, download, 0)
    }

    fn read_stream_with_offset(
        _username: &str,
        stream: &mut TcpStream,
        writer: &mut io::BufWriter<fs::File>,
        download: &Download,
        initial_bytes: usize,
    ) -> Result<usize, DownloadError> {
        let mut total_bytes = initial_bytes;
        let mut chunk_counter = 0usize;
        let mut last_update_time = Instant::now();
        let mut last_data_time = Instant::now();
        let mut read_buffer = [0u8; READ_BUFFER_SIZE];

        trace!("[download_peer:{}] reading stream data", _username);

        loop {
            match stream.read(&mut read_buffer) {
                Ok(0) => {
                    trace!(
                        "[download_peer:{}] connection closed by peer, {} bytes read",
                        _username, total_bytes
                    );
                    break;
                }
                Ok(bytes_read) => {
                    writer
                        .write_all(&read_buffer[..bytes_read])
                        .map_err(DownloadError::FileWriteError)?;
                    total_bytes += bytes_read;
                    chunk_counter += 1;
                    last_data_time = Instant::now();

                    // Check for manual cancellation after each chunk.
                    if download.cancel.load(Ordering::Relaxed) {
                        trace!("[download_peer:{}] cancelled by caller", _username);
                        return Err(DownloadError::Cancelled);
                    }

                    if chunk_counter.is_multiple_of(PROGRESS_UPDATE_CHUNKS) {
                        let elapsed = last_update_time.elapsed().as_secs_f64();
                        let speed = if elapsed > 0.0 {
                            (PROGRESS_UPDATE_CHUNKS * READ_BUFFER_SIZE) as f64 / elapsed
                        } else {
                            0.0
                        };
                        let _ = download.sender.send(DownloadStatus::InProgress {
                            bytes_downloaded: total_bytes as u64,
                            total_bytes: download.size,
                            speed_bytes_per_sec: speed,
                        });
                        last_update_time = Instant::now();
                    }

                    if total_bytes >= download.size as usize {
                        break;
                    }
                }
                Err(e) if e.kind() == io::ErrorKind::WouldBlock
                    || e.kind() == io::ErrorKind::TimedOut =>
                {
                    // The 1-second read timeout fired; check flags before retrying.
                    if download.cancel.load(Ordering::Relaxed) {
                        trace!("[download_peer:{}] cancelled by caller (stalled)", _username);
                        return Err(DownloadError::Cancelled);
                    }
                    if let Some(timeout) = download.progress_timeout
                        && last_update_time.elapsed() >= timeout
                    {
                        trace!(
                            "[download_peer:{}] no progress for {:?}, cancelling",
                            _username, timeout
                        );
                        return Err(DownloadError::NoProgressTimeout);
                    }
                    if last_data_time.elapsed() >= STALL_TIMEOUT {
                        return Err(DownloadError::StreamReadError(io::Error::new(
                            io::ErrorKind::TimedOut,
                            "peer stopped sending data",
                        )));
                    }
                    // Peer is briefly slow; keep waiting.
                }
                Err(e) => return Err(DownloadError::StreamReadError(e)),
            }
        }

        Ok(total_bytes - initial_bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::DownloadPeer;

    #[test]
    fn test_establish_connection_invalid_address() {
        let download_peer = DownloadPeer::new(
            "user".to_string(),
            "invalid-host".to_string(),
            9999,
            123,
            "own_user".to_string(),
        );
        let result = download_peer.establish_connection();
        assert!(result.is_err());
    }
}
