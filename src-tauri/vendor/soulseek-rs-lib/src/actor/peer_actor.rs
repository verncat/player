use crate::actor::{Actor, ConnectionState};
use crate::client::ClientOperation;
use crate::token::{PeerTransferToken, PierceToken};
use crate::dispatcher::MessageDispatcher;
use crate::message::peer::{
    FileSearchResponse, GetShareFileList, PeerInit, PlaceInQueueResponse, TransferRequest,
    TransferResponse, UploadFailedHandler,
};
use crate::message::server::MessageFactory;
#[allow(unused_imports)]
use crate::message::{Handlers, Message, MessageReader, MessageType};
use crate::path::SoulseekPath;
use crate::peer::Peer;
use crate::types::{Download, SearchResult, Transfer};
use crate::{debug, error, trace, warn};

use std::io;
use std::time::{Duration, Instant};

use tokio::net::TcpStream;
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};

#[derive(Debug)]
pub enum PeerCommand {
    QueueUpload(SoulseekPath),
    RequestTransfer(Download),
}

pub(crate) enum PeerSignal {
    ProcessRead,
    ConnectionEstablished(TcpStream),
    ConnectionFailed(io::Error),
    SendMessage(Message),
    FileSearchResult(SearchResult),
    TransferRequest(Transfer),
    TransferResponse {
        token: PeerTransferToken,
        allowed: bool,
        reason: Option<String>,
    },
    PlaceInQueueResponse {
        filename: SoulseekPath,
        place: u32,
    },
    SetUsername(String),
}

pub struct PeerActor {
    peer: Peer,
    stream: Option<TcpStream>,
    connection_state: ConnectionState,
    reader: MessageReader,
    client_channel: UnboundedSender<ClientOperation>,
    signal_tx: UnboundedSender<PeerSignal>,
    signal_rx: UnboundedReceiver<PeerSignal>,
    dispatcher: Option<MessageDispatcher<PeerSignal>>,
    queued_commands: Vec<PeerCommand>,
    #[allow(dead_code)]
    own_username: String,
    needs_handshake: bool,
    shared_folders: u32,
    shared_files: u32,
}

impl PeerActor {
    pub fn new(
        peer: Peer,
        stream: Option<TcpStream>,
        reader: Option<MessageReader>,
        client_channel: UnboundedSender<ClientOperation>,
        own_username: String,
        shared_folders: u32,
        shared_files: u32,
    ) -> Self {
        let needs_handshake = stream.is_none();
        let connection_state = if stream.is_some() {
            ConnectionState::Connected
        } else {
            ConnectionState::Disconnected
        };
        let (signal_tx, signal_rx) = mpsc::unbounded_channel::<PeerSignal>();

        Self {
            peer,
            stream,
            connection_state,
            reader: reader.unwrap_or_default(),
            client_channel,
            signal_tx,
            signal_rx,
            dispatcher: None,
            queued_commands: Vec::new(),
            own_username,
            needs_handshake,
            shared_folders,
            shared_files,
        }
    }

    fn initialize_dispatcher(&mut self) {
        let mut handlers = Handlers::new();
        handlers.register_handler(FileSearchResponse);
        handlers.register_handler(TransferRequest);
        handlers.register_handler(TransferResponse);
        handlers.register_handler(GetShareFileList {
            shared_folders: self.shared_folders,
            shared_files: self.shared_files,
        });
        handlers.register_handler(UploadFailedHandler);
        handlers.register_handler(PlaceInQueueResponse);
        handlers.register_handler(PeerInit);

        self.dispatcher = Some(MessageDispatcher::new(
            self.signal_tx.clone(),
            handlers,
        ));
    }

    fn drain_signals(&mut self) {
        let signals: Vec<PeerSignal> = {
            let mut sigs = Vec::new();
            while let Ok(sig) = self.signal_rx.try_recv() {
                sigs.push(sig);
            }
            sigs
        };
        for sig in signals {
            self.handle_signal(sig);
        }
    }

    fn handle_command(&mut self, cmd: PeerCommand) {
        match cmd {
            PeerCommand::QueueUpload(filename) => {
                let message = MessageFactory::build_queue_upload_message(filename.as_str());
                self.send_message(message);
            }
            PeerCommand::RequestTransfer(download) => {
                let message = MessageFactory::build_transfer_request_message(
                    download.filename.as_str(),
                    download.token.0,
                );
                self.send_message(message);
            }
        }
    }

    fn handle_signal(&mut self, sig: PeerSignal) {
        match sig {
            PeerSignal::SendMessage(message) => {
                self.send_message(message);
            }
            PeerSignal::FileSearchResult(file_search) => {
                self.client_channel
                    .send(ClientOperation::SearchResult(file_search))
                    .unwrap();
            }
            PeerSignal::TransferRequest(transfer) => {
                let username = self.peer.username.clone();
                debug!("[peer:{}] TransferRequest for {}", username, transfer.token);

                self.client_channel
                    .send(ClientOperation::UpdateDownloadTokens(
                        transfer.clone(),
                        username.clone(),
                    ))
                    .unwrap();

                let transfer_response = MessageFactory::build_transfer_response_message(transfer);
                self.send_message(transfer_response);
            }
            PeerSignal::TransferResponse {
                token,
                allowed,
                reason,
            } => {
                let _username = self.peer.username.clone();
                debug!(
                    "[peer:{}] transfer response token: {} allowed: {}",
                    _username, token, allowed
                );

                if !allowed {
                    debug!(
                        "[peer:{}] Transfer rejected: {:?} - token {}, resetting timeout...",
                        _username, reason, token
                    );
                    if let Err(_e) = self.client_channel.send(ClientOperation::TransferRejected {
                        token,
                        reason,
                    }) {
                        error!("[peer:{}] Failed to send TransferRejected: {}", _username, _e);
                    }
                } else {
                    debug!(
                        "[peer:{}] Transfer allowed, ready to connect with token {:}",
                        _username, token
                    );
                    if let Err(_e) = self.client_channel.send(ClientOperation::DownloadFromPeer(
                        token,
                        self.peer.clone(),
                        allowed,
                    )) {
                        error!("[peer:{}] Failed to send DownloadFromPeer: {}", _username, _e);
                    }
                }
            }
            PeerSignal::PlaceInQueueResponse { filename, place } => {
                let _username = self.peer.username.clone();
                debug!(
                    "[peer:{}] Place in queue response - file: {}, place: {}",
                    _username, filename, place
                );
                if let Err(_e) = self.client_channel.send(ClientOperation::QueuePositionUpdated {
                    username: _username.clone(),
                    filename,
                    place,
                }) {
                    error!("[peer:{}] Failed to send QueuePositionUpdated: {}", _username, _e);
                }
            }
            PeerSignal::SetUsername(username) => {
                trace!("[peer:{}] SetUsername: {}", self.peer.username, username);
                self.peer.username = username;
            }
            PeerSignal::ProcessRead => {
                self.process_read();
            }
            PeerSignal::ConnectionEstablished(stream) => {
                self.stream = Some(stream);
                self.connection_state = ConnectionState::Connected;
                self.on_connection_established();
            }
            PeerSignal::ConnectionFailed(e) => {
                self.disconnect(Some(e));
            }
        }
    }

    fn process_read(&mut self) {
        if self.reader.buffer_len() > 0 {
            self.extract_and_process_messages();
        }

        {
            let stream = match self.stream.as_ref() {
                Some(s) => s,
                None => return,
            };

            let mut temp_buffer = [0u8; 1024];
            match stream.try_read(&mut temp_buffer) {
                Ok(0) => {
                    // EOF — remote closed the connection cleanly.
                    trace!("[peer:{}] EOF, remote closed connection", self.peer.username);
                    self.disconnect(None);
                    return;
                }
                Ok(n) => {
                    self.reader.push_bytes(&temp_buffer[..n]);
                }
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {}
                Err(ref e) if e.kind() == io::ErrorKind::TimedOut => {
                    debug!(
                        "Read operation timed out for peer actor {:}:{:}",
                        self.peer.host, self.peer.port
                    );
                }
                Err(e) => {
                    let _username = self.peer.username.clone();
                    error!(
                        "[peer:{}] Error reading from peer: {} (kind: {:?}). Disconnecting.",
                        _username,
                        e,
                        e.kind()
                    );
                    self.disconnect(Some(e));
                    return;
                }
            }
        }
        self.extract_and_process_messages();
    }

    fn extract_and_process_messages(&mut self) {
        let _username = self.peer.username.clone();
        let mut _extracted_count = 0;
        loop {
            match self.reader.extract_message() {
                Ok(Some(mut message)) => {
                    _extracted_count += 1;
                    trace!(
                        "[peer:{}] ← Message #{}: {:?}",
                        _username,
                        _extracted_count,
                        message
                            .get_message_name(MessageType::Peer, message.get_message_code() as u32)
                            .map_err(|e| e.to_string())
                    );
                    if let Some(ref dispatcher) = self.dispatcher {
                        dispatcher.dispatch(&mut message);
                    } else {
                        warn!("[peer:{}] No dispatcher available!", _username);
                    }
                }
                Err(e) => {
                    warn!(
                        "[peer:{}] Error extracting message: {}. Disconnecting peer.",
                        _username, e
                    );
                    self.disconnect(Some(e));
                    return;
                }
                Ok(None) => {
                    break;
                }
            }
        }

        self.drain_signals();
    }

    fn send_message(&mut self, message: Message) {
        let stream = match self.stream.as_ref() {
            Some(s) => s,
            None => {
                error!("Cannot send message: stream is None");
                return;
            }
        };

        let _username = self.peer.username.clone();
        trace!(
            "[peer:{}] ➡ {:?}",
            _username,
            message
                .get_message_name(
                    MessageType::Peer,
                    u32::from_le_bytes(message.get_slice(0, 4).try_into().unwrap())
                )
                .map_err(|e| e.to_string())
        );

        let buf = message.get_buffer();
        match stream.try_write(&buf) {
            Ok(_n) if _n == buf.len() => {}
            Ok(_n) => {
                error!(
                    "[peer:{}] Partial write: {} of {} bytes",
                    _username,
                    _n,
                    buf.len()
                );
            }
            Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                warn!("[peer:{}] Write would block, message may be lost", _username);
            }
            Err(e) => {
                error!(
                    "[peer:{}] Error writing message: {}. Disconnecting.",
                    _username, e
                );
                self.disconnect(Some(e));
            }
        }
    }

    fn disconnect(&mut self, error: Option<io::Error>) {
        debug!("[peer:{}] disconnect", self.peer.username);
        self.stream.take();
        if let Err(_e) = self.client_channel.send(ClientOperation::PeerDisconnected(
            self.peer.username.clone(),
            error.map(Into::into),
        )) {
            error!("Failed to send disconnect notification: {}", _e);
        }
    }

    fn initiate_connection(&mut self) -> bool {
        let _username = self.peer.username.clone();
        let host = self.peer.host.clone();
        let port = self.peer.port;

        let socket_addr = format!("{}:{}", host, port).parse::<std::net::SocketAddr>();

        match socket_addr {
            Ok(addr) => {
                self.connection_state = ConnectionState::Connecting {
                    since: Instant::now(),
                };

                let signal_tx = self.signal_tx.clone();
                tokio::spawn(async move {
                    let timeout = Duration::from_secs(5);
                    let result = tokio::time::timeout(timeout, TcpStream::connect(addr)).await;

                    match result {
                        Ok(Ok(stream)) => {
                            stream.set_nodelay(true).ok();
                            let _ = signal_tx.send(PeerSignal::ConnectionEstablished(stream));
                        }
                        Ok(Err(e)) => {
                            let _ = signal_tx.send(PeerSignal::ConnectionFailed(e));
                        }
                        Err(_) => {
                            let _ = signal_tx.send(PeerSignal::ConnectionFailed(io::Error::new(
                                io::ErrorKind::TimedOut,
                                "connection timed out",
                            )));
                        }
                    }
                });
                true
            }
            Err(e) => {
                error!(
                    "[peer:{}] Invalid socket address {}:{} - {}",
                    _username, host, port, e
                );
                self.disconnect(Some(io::Error::new(io::ErrorKind::InvalidInput, e)));
                false
            }
        }
    }

    fn check_connection_status(&mut self) {
        let ConnectionState::Connecting { since } = self.connection_state else {
            return;
        };

        if since.elapsed() > Duration::from_secs(10) {
            error!(
                "[peer:{}] Connection timeout after 10 seconds",
                self.peer.username
            );
            self.disconnect(Some(io::Error::new(
                io::ErrorKind::TimedOut,
                "Connection timeout",
            )));
        }
    }

    fn on_connection_established(&mut self) {
        let _username = self.peer.username.clone();
        let token = self.peer.token.unwrap_or(PierceToken(0));

        let Some(ref stream) = self.stream else {
            return;
        };

        if self.needs_handshake {
            let handshake_msg = MessageFactory::build_pierce_firewall_message(token.0);
            match stream.try_write(&handshake_msg.get_buffer()) {
                Ok(_) => {
                    debug!(
                        "[peer:{}] Sent PierceFireWall handshake (token={})",
                        _username, token
                    );
                }
                Err(e) => {
                    error!(
                        "[peer:{}] Failed to send PierceFireWall handshake: {}",
                        _username, e
                    );
                    self.disconnect(Some(e));
                    return;
                }
            }
        }

        self.initialize_dispatcher();

        let queued = std::mem::take(&mut self.queued_commands);
        for cmd in queued {
            self.handle_command(cmd);
        }

        self.signal_tx.send(PeerSignal::ProcessRead).ok();
        self.process_read();
    }
}

impl Actor for PeerActor {
    type Message = PeerCommand;

    fn handle(&mut self, msg: Self::Message) {
        if !matches!(self.connection_state, ConnectionState::Connected) || self.stream.is_none() {
            self.queued_commands.push(msg);
            return;
        }
        self.handle_command(msg);
    }

    fn on_start(&mut self) {
        if self.stream.is_none() {
            self.initiate_connection();
        } else {
            self.connection_state = ConnectionState::Connected;
            self.on_connection_established();
        }
    }

    fn on_stop(&mut self) {
        trace!("[peer:{}] actor stopping", self.peer.username);
        self.disconnect(None);
    }

    fn tick(&mut self) {
        self.drain_signals();
        match self.connection_state {
            ConnectionState::Connecting { .. } => {
                self.check_connection_status();
            }
            ConnectionState::Connected => {
                if self.stream.is_some() {
                    self.process_read();
                }
            }
            ConnectionState::Disconnected => {}
        }
    }
}
