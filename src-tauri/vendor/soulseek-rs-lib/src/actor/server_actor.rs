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
use std::collections::HashMap;
use crate::message::server::ItemSimilarUsersHandler;
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

#[derive(Clone)]
pub struct UserMessage {
    pub id: u32,
    pub timestamp: u32,
    pub username: String,
    pub message: String,
    pub new_message: bool,
}
impl UserMessage {
    pub fn new(id: u32, timestamp: u32, username: String, message: String, new_message: bool) -> Self {
        Self { id, timestamp, username, message, new_message }
    }
}

/// Return true only after durable storage. False leaves the message unacknowledged.
pub type PrivateMessageHandler = std::sync::Arc<dyn Fn(&UserMessage) -> bool + Send + Sync>;
pub type SharedMessageHandler = std::sync::Arc<std::sync::Mutex<Option<PrivateMessageHandler>>>;

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
    SendPrivateMessage {
        username: String,
        body: String,
        response: tokio::sync::oneshot::Sender<Result<(), SoulseekRs>>,
    },
    SetDiscoveryInterest {
        previous: Option<String>,
        current: Option<String>,
        response: tokio::sync::oneshot::Sender<Result<(), SoulseekRs>>,
    },
    FindInterestUsers {
        item: String,
        response: tokio::sync::oneshot::Sender<Result<Vec<String>, SoulseekRs>>,
    },
}

/// Internal signals produced by wire message handlers or actor self-sends.
pub(crate) enum ServerSignal {
    ProcessRead,
    InterestUsers { item: String, users: Vec<String> },
    PrivateMessage(UserMessage),
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

struct InterestQuery {
    deadline: Instant,
    responses: Vec<tokio::sync::oneshot::Sender<Result<Vec<String>, SoulseekRs>>>,
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
    discovery_interest: Option<String>,
    pub(crate) private_message_handler: SharedMessageHandler,
    interest_queries: HashMap<String, InterestQuery>,
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
            discovery_interest: None,
            private_message_handler: Default::default(),
            interest_queries: HashMap::new(),
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
                            #[cfg(target_os = "windows")]
                            let ka = socket2::TcpKeepalive::new()
                                .with_time(*idle)
                                .with_interval(*interval);
                            #[cfg(not(target_os = "windows"))]
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

    fn write_interest(&mut self, code: u32, item: &str) -> Result<(), SoulseekRs> {
        self.write_packet(Message::new().write_int32(code).write_string(item).get_buffer())
    }

    fn write_packet(&mut self, packet: Vec<u8>) -> Result<(), SoulseekRs> {
        let ServerConnection::Connected { ref stream, .. } = self.connection else {
            return Err(SoulseekRs::NotConnected);
        };
        match stream.try_write(&packet) {
            Ok(written) if written == packet.len() => Ok(()),
            Ok(_) => {
                self.disconnect_with_error(io::Error::new(io::ErrorKind::WriteZero, "Partial server write"));
                Err(SoulseekRs::ConnectionClosed)
            }
            Err(error) => Err(SoulseekRs::NetworkError(error)),
        }
    }

    fn handle_command(&mut self, cmd: ServerCommand) {
        match cmd {
            ServerCommand::SendPrivateMessage { username, body, response } => {
                if response.is_closed() { return; }
                let packet = Message::new().write_int32(22).write_string(&username).write_string(&body).get_buffer();
                let result = self.write_packet(packet);
                let _ = response.send(result);
            }
            ServerCommand::SetDiscoveryInterest { previous, current, response } => {
                let result = (|| {
                    for old in [previous.as_ref(), self.discovery_interest.as_ref()].into_iter().flatten()
                        .filter(|old| Some(*old) != current.as_ref()).cloned().collect::<std::collections::HashSet<_>>() {
                        self.write_interest(52, &old)?;
                    }
                    if self.discovery_interest != current {
                        if let Some(item) = &current { self.write_interest(51, item)?; }
                        self.discovery_interest = current;
                    }
                    Ok(())
                })();
                let _ = response.send(result);
            }
            ServerCommand::FindInterestUsers { item, response } => {
                if let Some(query) = self.interest_queries.get_mut(&item) {
                    if query.responses.len() < 16 { query.responses.push(response); }
                    else { let _ = response.send(Err(SoulseekRs::Timeout)); }
                } else if self.interest_queries.len() < 32 {
                    self.queue_message(Message::new().write_int32(112).write_string(&item).clone());
                    self.interest_queries.insert(item, InterestQuery {
                        deadline: Instant::now() + Duration::from_secs(8), responses: vec![response],
                    });
                } else { let _ = response.send(Err(SoulseekRs::Timeout)); }
            }
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
            ServerSignal::PrivateMessage(message) => {
                let handler = self.private_message_handler.lock().ok().and_then(|handler| handler.clone());
                if handler.is_some_and(|store| store(&message)) {
                    // Persist first, acknowledge second. A replay is deduplicated by the application.
                    self.queue_message(Message::new().write_int32(23).write_int32(message.id).clone());
                }
            }
            ServerSignal::InterestUsers { item, users } => {
                if let Some(query) = self.interest_queries.remove(&item) {
                    for response in query.responses { let _ = response.send(Ok(users.clone())); }
                }
            }
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
                    if let Some(item) = &self.discovery_interest {
                        self.queue_message(Message::new().write_int32(51).write_string(item).clone());
                    }
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
                    self.disconnect_with_error(io::Error::new(io::ErrorKind::UnexpectedEof, "Server closed the connection"));
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
        for (_, query) in self.interest_queries.drain() {
            for response in query.responses { let _ = response.send(Err(SoulseekRs::ConnectionClosed)); }
        }

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
        handlers.register_handler(ItemSimilarUsersHandler);
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
        let msg = if !matches!(self.connection, ServerConnection::Connected { .. })
            || !matches!(self.login_state, LoginState::LoggedIn { .. }) {
            match msg {
                ServerCommand::SendPrivateMessage { response, .. } => {
                    let _ = response.send(Err(SoulseekRs::NotConnected));
                    return;
                }
                ServerCommand::SetDiscoveryInterest { response, .. } => {
                    let _ = response.send(Err(SoulseekRs::NotConnected));
                    return;
                }
                ServerCommand::FindInterestUsers { response, .. } => {
                    let _ = response.send(Err(SoulseekRs::NotConnected));
                    return;
                }
                other => other,
            }
        } else { msg };
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
        self.interest_queries.retain(|_, query| {
            if Instant::now() < query.deadline && query.responses.iter().any(|r| !r.is_closed()) { return true; }
            for response in query.responses.drain(..) { let _ = response.send(Err(SoulseekRs::Timeout)); }
            false
        });

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

#[cfg(test)]
mod interest_tests {
    use super::*;

    fn actor() -> ServerActor {
        let (client, _) = mpsc::unbounded_channel();
        ServerActor::new(PeerAddress::new("127.0.0.1".into(), 9), client, ServerActorConfig {
            listen_port: 0, enable_listen: false, shared_folders: 0, shared_files: 0,
            tcp_keepalive: KeepAliveSettings::Disabled, reconnect_settings: ReconnectSettings::Disabled,
        })
    }

    #[test]
    fn private_message_is_acknowledged_only_after_storage_succeeds() {
        let mut actor = actor();
        let message = UserMessage::new(77, 42, "alice".into(), "Hello".into(), true);
        *actor.private_message_handler.lock().unwrap() = Some(std::sync::Arc::new(|_| false));
        actor.handle_signal(ServerSignal::PrivateMessage(message.clone()));
        assert!(actor.signal_rx.try_recv().is_err());
        *actor.private_message_handler.lock().unwrap() = Some(std::sync::Arc::new(|_| true));
        actor.handle_signal(ServerSignal::PrivateMessage(message));
        let ServerSignal::SendMessage(mut ack) = actor.signal_rx.try_recv().unwrap() else { panic!("expected acknowledgement") };
        assert_eq!(ack.read_int32(), 23);
        assert_eq!(ack.read_int32(), 77);
    }

    #[test]
    fn relogin_replays_the_current_discovery_marker() {
        let mut actor = actor();
        actor.discovery_interest = Some("adhdplayer-v1-current".into());
        actor.handle_signal(ServerSignal::LoginStatus(true));
        let mut markers = Vec::new();
        while let Ok(signal) = actor.signal_rx.try_recv() {
            if let ServerSignal::SendMessage(mut message) = signal {
                if message.read_int32() == 51 { markers.push(message.read_string()); }
            }
        }
        assert_eq!(markers, vec!["adhdplayer-v1-current"]);
    }

    #[tokio::test]
    async fn pending_interest_times_out_instead_of_returning_no_matches() {
        let mut actor = actor();
        let (response, receiver) = tokio::sync::oneshot::channel();
        actor.interest_queries.insert("marker".into(), InterestQuery {
            deadline: Instant::now() - Duration::from_secs(1), responses: vec![response],
        });
        actor.tick();
        assert!(matches!(receiver.await.unwrap(), Err(SoulseekRs::Timeout)));
        assert!(actor.interest_queries.is_empty());
    }

    #[tokio::test]
    async fn disconnected_interest_queries_fail_and_can_be_retried() {
        let mut actor = actor();
        let (response, receiver) = tokio::sync::oneshot::channel();
        actor.interest_queries.insert("marker".into(), InterestQuery {
            deadline: Instant::now() + Duration::from_secs(5), responses: vec![response],
        });
        actor.disconnect_with_error(io::Error::new(io::ErrorKind::ConnectionReset, "test"));
        assert!(matches!(receiver.await.unwrap(), Err(SoulseekRs::ConnectionClosed)));
        assert!(actor.interest_queries.is_empty());
    }
}
