use crate::actor::Actor;
use crate::client::{ClientOperation, KeepAliveSettings, ReconnectSettings};
use crate::dispatcher::MessageDispatcher;
use crate::message::server::ConnectToPeerHandler;
use crate::message::server::ExcludedSearchPhrasesHandler;
use crate::message::server::FileSearchHandler;
use crate::message::server::GetPeerAddressHandler;
use crate::message::server::LoginHandler;
use crate::message::server::MessageFactory;
use crate::message::server::MessageUser;
use crate::message::server::ParentMinSpeedHandler;
use crate::message::server::ParentSpeedRatioHandler;
use crate::message::server::PrivilegedUsersHandler;
use crate::message::server::RoomListHandler;
use crate::message::server::WishListIntervalHandler;
use crate::message::Handlers;
#[allow(unused_imports)]
use crate::message::MessageType;
use crate::message::{Message, MessageReader};
use crate::peer::ConnectionType;
use crate::peer::Peer;

use std::io;
use std::net::ToSocketAddrs;
use std::time::{Duration, Instant};

use tokio::net::TcpStream;
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};

use crate::token::{PierceToken, SearchToken};
use crate::{SoulseekRs, debug, error, info, trace, warn};

#[derive(Debug, Clone)]
pub struct PeerAddress {
    host: String,
    port: u16,
}

impl PeerAddress {
    pub fn new(host: String, port: u16) -> Self {
        Self { host, port }
    }

    pub fn get_host(&self) -> &str {
        &self.host
    }

    pub fn get_port(&self) -> u16 {
        self.port
    }
}

impl std::fmt::Display for PeerAddress {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "{}:{}", self.host, self.port)
    }
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct UserMessage {
    id: u32,
    timestamp: u32,
    username: String,
    message: String,
    new_message: bool,
}
impl UserMessage {
    pub fn new(
        id: u32,
        timestamp: u32,
        username: String,
        message: String,
        new_message: bool,
    ) -> Self {
        Self {
            id,
            timestamp,
            username,
            message,
            new_message,
        }
    }
    pub fn print(&self) {
        debug!(
            "Timestamp: {}. User: {}, Id: #{}, New message: {} Message: {}",
            self.timestamp, self.username, self.id, self.new_message, self.message
        );
    }
}

/// Commands sent from external callers (client, connected_worker) into the actor.
#[derive(Debug)]
pub enum ServerCommand {
    Login {
        username: String,
        password: String,
        response: tokio::sync::oneshot::Sender<Result<bool, SoulseekRs>>,
    },
    FileSearch {
        token: SearchToken,
        query: String,
    },
    PierceFirewall(PierceToken),
    GetPeerAddress(String),
}

/// Internal signals produced by wire message handlers or actor self-sends.
pub(crate) enum ServerSignal {
    ProcessRead,
    LoginStatus(bool),
    SendMessage(Message),
    ConnectToPeer(Peer),
    GetPeerAddressResponse {
        username: String,
        host: String,
        port: u32,
        obfuscation_type: u32,
        obfuscated_port: u16,
    },
}

struct Dispatcher {
    inner: MessageDispatcher<ServerSignal>,
}

enum ServerConnection {
    Disconnected {
        reconnect_attempt: u32,
        last_disconnect: Option<Instant>,
    },
    Connecting {
        stream: TcpStream,
        since: Instant,
        reconnect_attempt: u32,
    },
    Connected {
        stream: TcpStream,
        dispatcher: Dispatcher,
    },
}

enum LoginState {
    NotAttempted,
    Pending {
        credentials: (String, String),
        response: tokio::sync::oneshot::Sender<Result<bool, SoulseekRs>>,
        deadline: Instant,
    },
    LoggedIn {
        credentials: (String, String),
    },
}

pub struct ServerActorConfig {
    pub listen_port: u32,
    pub enable_listen: bool,
    pub shared_folders: u32,
    pub shared_files: u32,
    pub tcp_keepalive: KeepAliveSettings,
    pub reconnect_settings: ReconnectSettings,
}

pub struct ServerActor {
    address: PeerAddress,
    listen_port: u32,
    enable_listen: bool,
    shared_folders: u32,
    shared_files: u32,
    connection: ServerConnection,
    login_state: LoginState,
    reader: MessageReader,
    client_channel: UnboundedSender<ClientOperation>,
    signal_tx: UnboundedSender<ServerSignal>,
    signal_rx: UnboundedReceiver<ServerSignal>,
    queued_messages: Vec<ServerCommand>,
    reconnect_settings: ReconnectSettings,
    tcp_keepalive: KeepAliveSettings,
}

impl ServerActor {
    pub fn new(
        address: PeerAddress,
        client_channel: UnboundedSender<ClientOperation>,
        config: ServerActorConfig,
    ) -> Self {
        let (signal_tx, signal_rx) = mpsc::unbounded_channel::<ServerSignal>();
        Self {
            address,
            listen_port: config.listen_port,
            enable_listen: config.enable_listen,
            shared_folders: config.shared_folders,
            shared_files: config.shared_files,
            connection: ServerConnection::Disconnected { reconnect_attempt: 0, last_disconnect: None },
            login_state: LoginState::NotAttempted,
            reader: MessageReader::new(),
            client_channel,
            signal_tx,
            signal_rx,
            queued_messages: Vec::new(),
            reconnect_settings: config.reconnect_settings,
            tcp_keepalive: config.tcp_keepalive,
        }
    }

    pub fn get_address(&self) -> &PeerAddress {
        &self.address
    }

    fn current_reconnect_attempt(&self) -> u32 {
        match &self.connection {
            ServerConnection::Disconnected { reconnect_attempt, .. } => *reconnect_attempt,
            ServerConnection::Connecting { reconnect_attempt, .. } => *reconnect_attempt,
            ServerConnection::Connected { .. } => 0,
        }
    }

    fn initiate_connection(&mut self) -> bool {
        let host = self.address.host.clone();
        let port = self.address.port;

        let addr_str = format!("{}:{}", host, port);

        let mut socket_addrs = match addr_str.to_socket_addrs() {
            Ok(addrs) => addrs,
            Err(e) => {
                error!("[server] Failed to resolve address: {}", e);
                self.disconnect_with_error(io::Error::new(io::ErrorKind::InvalidInput, e));
                return false;
            }
        };

        let socket_addr = socket_addrs.next();

        match socket_addr {
            Some(addr) => {
                match std::net::TcpStream::connect(addr) {
                    Ok(std_stream) => {
                        let socket = socket2::Socket::from(std_stream);
                        if let KeepAliveSettings::Enabled {
                            idle,
                            interval,
                            count,
                            ..
                        } = &self.tcp_keepalive
                        {
                            let ka = socket2::TcpKeepalive::new()
                                .with_time(*idle)
                                .with_interval(*interval)
                                .with_retries(*count);
                            socket.set_tcp_keepalive(&ka).ok();
                        }
                        let std_stream: std::net::TcpStream = socket.into();

                        std_stream.set_nodelay(true).ok();
                        std_stream.set_nonblocking(true).ok();

                        match TcpStream::from_std(std_stream) {
                            Ok(stream) => {
                                let reconnect_attempt = self.current_reconnect_attempt();
                                self.connection = ServerConnection::Connecting {
                                    stream,
                                    since: Instant::now(),
                                    reconnect_attempt,
                                };
                                true
                            }
                            Err(e) => {
                                error!("[server] Failed to convert to tokio TcpStream: {}", e);
                                self.disconnect_with_error(e);
                                false
                            }
                        }
                    }
                    Err(e) => {
                        self.disconnect_with_error(e);
                        false
                    }
                }
            }
            None => {
                let error_msg = format!("No socket addresses found for {}:{}", host, port);
                error!("[server] {}", error_msg);
                self.disconnect_with_error(io::Error::new(io::ErrorKind::InvalidInput, error_msg));
                false
            }
        }
    }

    fn drain_signals(&mut self) {
        let signals: Vec<ServerSignal> = {
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

    fn handle_command(&mut self, cmd: ServerCommand) {
        match cmd {
            ServerCommand::Login {
                username,
                password,
                response,
            } => {
                self.queue_message(MessageFactory::build_login_message(&username, &password));
                self.login_state = LoginState::Pending {
                    credentials: (username, password),
                    response,
                    deadline: Instant::now() + Duration::from_secs(5),
                };
            }
            ServerCommand::FileSearch { token, query } => {
                self.queue_message(MessageFactory::build_file_search_message(token.0, &query));
            }
            ServerCommand::PierceFirewall(token) => {
                self.queue_message(MessageFactory::build_pierce_firewall_message(token.0));
            }
            ServerCommand::GetPeerAddress(username) => {
                self.queue_message(MessageFactory::build_get_peer_address(&username));
            }
        }
    }

    fn handle_signal(&mut self, sig: ServerSignal) {
        match sig {
            ServerSignal::ConnectToPeer(peer) => {
                if let Some(op) = match peer.connection_type {
                    ConnectionType::P | ConnectionType::F => {
                        Some(ClientOperation::ConnectToPeer(peer.clone()))
                    }
                    ConnectionType::D => None,
                }
                    && let Err(_e) = self.client_channel.send(op)
                {
                    error!("[server] Failed to send ConnectToPeer: {}", _e);
                }
            }
            ServerSignal::LoginStatus(logged_in) => {
                match std::mem::replace(&mut self.login_state, LoginState::NotAttempted) {
                    LoginState::Pending { credentials, response, .. } => {
                        if logged_in {
                            let _ = response.send(Ok(true));
                            self.login_state = LoginState::LoggedIn { credentials };
                        } else {
                            let _ = response.send(Err(SoulseekRs::AuthenticationFailed));
                        }
                    }
                    other => {
                        self.login_state = other;
                    }
                }

                if logged_in {
                    if let Err(_e) = self.client_channel.send(ClientOperation::LoginSucceeded) {
                        error!("[server] Failed to send LoginSucceeded: {}", _e);
                    }

                    self.queue_message(MessageFactory::build_shared_folders_message(
                        self.shared_folders,
                        self.shared_files,
                    ));
                    self.queue_message(MessageFactory::build_no_parent_message());
                    self.queue_message(MessageFactory::build_set_status_message(2));
                    if self.enable_listen {
                        self.queue_message(MessageFactory::build_set_wait_port_message(
                            self.listen_port,
                        ));
                    }
                }
            }
            ServerSignal::SendMessage(message) => {
                self.send_message(message);
            }
            ServerSignal::GetPeerAddressResponse {
                username,
                host,
                port,
                obfuscation_type,
                obfuscated_port,
            } => {
                debug!(
                    "[server] Received GetPeerAddress response for {}: {}:{} (obf_type: {}, obf_port: {})",
                    username, host, port, obfuscation_type, obfuscated_port
                );

                if let Err(_e) = self
                    .client_channel
                    .send(ClientOperation::GetPeerAddressResponse {
                        username,
                        host,
                        port,
                        obfuscation_type,
                        obfuscated_port,
                    })
                {
                    error!(
                        "[server] Error forwarding GetPeerAddress response to client: {}",
                        _e
                    );
                }
            }
            ServerSignal::ProcessRead => {
                self.process_read();
            }
        }
    }

    fn process_read(&mut self) {
        if self.reader.buffer_len() > 0 {
            self.extract_and_process_messages();
        }

        {
            let ServerConnection::Connected { ref stream, .. } = self.connection else {
                return;
            };

            let mut temp_buffer = [0u8; 1024];
            match stream.try_read(&mut temp_buffer) {
                Ok(0) => {
                    return;
                }
                Ok(n) => {
                    self.reader.push_bytes(&temp_buffer[..n]);
                }
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {}
                Err(ref e) if e.kind() == io::ErrorKind::TimedOut => {
                    debug!("[server] Read operation timed out",);
                }
                Err(e) => {
                    error!(
                        "[server] Error reading from server: {} (kind: {:?}). Disconnecting.",
                        e,
                        e.kind()
                    );
                    self.disconnect_with_error(e);
                    return;
                }
            }
        }
        self.extract_and_process_messages();
    }

    fn extract_and_process_messages(&mut self) {
        let mut _extracted_count = 0;
        loop {
            match self.reader.extract_message() {
                Ok(Some(mut message)) => {
                    _extracted_count += 1;
                    trace!(
                        "[server] ← Message #{}: {:?}",
                        _extracted_count,
                        message
                            .get_message_name(
                                MessageType::Server,
                                message.get_message_code() as u32
                            )
                            .map_err(|e| e.to_string())
                    );
                    if let ServerConnection::Connected { dispatcher, .. } = &self.connection {
                        dispatcher.inner.dispatch(&mut message);
                    } else {
                        warn!("[server] No dispatcher available!");
                    }
                }
                Err(e) => {
                    warn!("[server] Error extracting message: {}. Disconnecting.", e);
                    self.disconnect_with_error(e);
                    return;
                }
                Ok(None) => {
                    break;
                }
            }
        }

        self.drain_signals();
    }

    fn queue_message(&mut self, message: Message) {
        let _ = self.signal_tx.send(ServerSignal::SendMessage(message));
    }

    fn send_message(&mut self, message: Message) {
        let ServerConnection::Connected { ref stream, .. } = self.connection else {
            error!("[server] Cannot send message: not connected");
            return;
        };

        trace!(
            "[server] ➡ {:?}",
            message
                .get_message_name(
                    MessageType::Server,
                    u32::from_le_bytes(message.get_slice(0, 4).try_into().unwrap())
                )
                .map_err(|e| e.to_string())
        );

        let buf = message.get_buffer();
        match stream.try_write(&buf) {
            Ok(_n) if _n == buf.len() => {}
            Ok(_n) => {
                error!("[server] Partial write: {} of {} bytes", _n, buf.len());
            }
            Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                warn!("[server] Write would block, message may be lost");
            }
            Err(e) => {
                error!("[server] Error writing message: {}. Disconnecting.", e);
                self.disconnect_with_error(e);
            }
        }
    }

    fn disconnect_with_error(&mut self, _error: io::Error) {
        debug!("[server] disconnect");

        let new_reconnect_attempt = match &self.connection {
            ServerConnection::Connected { .. } => 1,
            ServerConnection::Connecting { reconnect_attempt, .. } => reconnect_attempt + 1,
            ServerConnection::Disconnected { .. } => return,
        };
        self.connection = ServerConnection::Disconnected {
            reconnect_attempt: new_reconnect_attempt,
            last_disconnect: Some(Instant::now()),
        };

        if let LoginState::Pending { credentials, response, .. } =
            std::mem::replace(&mut self.login_state, LoginState::NotAttempted)
        {
            let _ = response.send(Err(SoulseekRs::ConnectionClosed));
            self.login_state = LoginState::LoggedIn { credentials };
        }

        if let Err(_e) = self
            .client_channel
            .send(ClientOperation::ServerDisconnected)
        {
            error!("[server] Failed to send ServerDisconnected: {}", _e);
        }
    }

    fn check_connection_status(&mut self) {
        let ServerConnection::Connecting { since, .. } = self.connection else {
            return;
        };

        if since.elapsed() > Duration::from_secs(20) {
            error!("[server] Connection timeout after 20 seconds");
            self.disconnect_with_error(io::Error::new(
                io::ErrorKind::TimedOut,
                "Connection timeout",
            ));
            return;
        }

        let ServerConnection::Connecting { ref stream, .. } = self.connection else {
            return;
        };
        let mut buf = [0u8; 1];
        match stream.try_read(&mut buf) {
            Ok(_) => {
                if buf[0] != 0 {
                    self.reader.push_bytes(&buf[..1]);
                }
                self.on_connection_established();
            }
            Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                self.on_connection_established();
            }
            Err(e) => {
                error!("[server] Connection failed: {}", e);
                self.disconnect_with_error(e);
            }
        }
    }

    fn on_connection_established(&mut self) {
        let stream = match std::mem::replace(
            &mut self.connection,
            ServerConnection::Disconnected { reconnect_attempt: 0, last_disconnect: None },
        ) {
            ServerConnection::Connecting { stream, .. } => stream,
            other => {
                self.connection = other;
                error!("[server] on_connection_established called in wrong state");
                return;
            }
        };

        let mut handlers = Handlers::new();
        handlers.register_handler(LoginHandler);
        handlers.register_handler(RoomListHandler);
        handlers.register_handler(ExcludedSearchPhrasesHandler);
        handlers.register_handler(PrivilegedUsersHandler);
        handlers.register_handler(MessageUser);
        handlers.register_handler(WishListIntervalHandler);
        handlers.register_handler(ParentMinSpeedHandler);
        handlers.register_handler(ParentSpeedRatioHandler);
        handlers.register_handler(FileSearchHandler);
        handlers.register_handler(GetPeerAddressHandler);
        handlers.register_handler(ConnectToPeerHandler);

        let dispatcher = Dispatcher {
            inner: MessageDispatcher::new(self.signal_tx.clone(), handlers),
        };

        self.connection = ServerConnection::Connected { stream, dispatcher };

        // On reconnect (previously logged in), auto-queue login with stored credentials.
        if let LoginState::LoggedIn { credentials: (ref u, ref p) } = self.login_state {
            let (username, password) = (u.clone(), p.clone());
            info!("[server] Reconnect: auto-queuing login for {}", username);
            self.queue_message(MessageFactory::build_login_message(&username, &password));
        }

        let queued = std::mem::take(&mut self.queued_messages);
        for cmd in queued {
            self.handle_command(cmd);
        }

        self.signal_tx.send(ServerSignal::ProcessRead).ok();
        self.process_read();
    }

    fn maybe_reconnect(&mut self) {
        if !matches!(self.login_state, LoginState::LoggedIn { .. }) {
            return;
        }

        let (reconnect_attempt, last_disconnect) = match &self.connection {
            ServerConnection::Disconnected { reconnect_attempt, last_disconnect } => {
                (*reconnect_attempt, *last_disconnect)
            }
            _ => return,
        };

        let Some(last_disconnect) = last_disconnect else {
            return;
        };

        let backoff = match &self.reconnect_settings {
            ReconnectSettings::Disabled => return,
            ReconnectSettings::EnabledExponentialBackoff {
                min_delay,
                max_delay,
                max_attempts,
            } => {
                if let Some(max) = max_attempts
                    && reconnect_attempt > *max
                {
                    warn!(
                        "[server] Max reconnect attempts ({}) reached, giving up",
                        max
                    );
                    return;
                }
                let exp = reconnect_attempt.saturating_sub(1);
                let factor = 1u64.checked_shl(exp).unwrap_or(u64::MAX);
                let delay_secs = (min_delay.as_secs()).saturating_mul(factor);
                Duration::from_secs(delay_secs.min(max_delay.as_secs()))
            }
        };

        if last_disconnect.elapsed() < backoff {
            return;
        }

        info!(
            "[server] Attempting reconnect (attempt {}, backoff {}s)...",
            reconnect_attempt,
            backoff.as_secs()
        );
        self.initiate_connection();
    }
}

impl Actor for ServerActor {
    type Message = ServerCommand;

    fn handle(&mut self, msg: Self::Message) {
        if !matches!(self.connection, ServerConnection::Connected { .. }) {
            self.queued_messages.push(msg);
            return;
        }
        self.handle_command(msg);
    }

    fn on_start(&mut self) {
        if matches!(self.connection, ServerConnection::Disconnected { .. }) {
            self.initiate_connection();
        } else {
            self.on_connection_established();
        }
    }

    fn on_stop(&mut self) {
        trace!("[server] actor stopping");
        self.connection =
            ServerConnection::Disconnected { reconnect_attempt: 0, last_disconnect: None };
    }

    fn tick(&mut self) {
        self.drain_signals();

        // Time out any pending login wait.
        if let LoginState::Pending { ref deadline, .. } = self.login_state
            && Instant::now() >= *deadline
            && let LoginState::Pending { response, .. } =
                std::mem::replace(&mut self.login_state, LoginState::NotAttempted)
        {
            let _ = response.send(Err(SoulseekRs::Timeout));
        }

        match &self.connection {
            ServerConnection::Connecting { .. } => {
                self.check_connection_status();
            }
            ServerConnection::Connected { .. } => {
                self.process_read();
            }
            ServerConnection::Disconnected { .. } => {
                self.maybe_reconnect();
            }
        }
    }
}
