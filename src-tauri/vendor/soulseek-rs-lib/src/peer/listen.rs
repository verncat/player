use std::io;
use std::sync::Arc;
use std::time::Duration;

use tokio::io::AsyncReadExt;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc::UnboundedSender;
use tokio::sync::oneshot;

use crate::client::{ClientContext, ClientOperation};
use crate::message::{Message, MessageReader};
use crate::peer::download_peer::spawn_direct_download;
use crate::peer::{ConnectionType, Peer};
use crate::token::PeerTransferToken;
use crate::{debug, error, info, trace};

const PEER_INIT_MESSAGE_CODE: u8 = 1;
const PIERCE_FIREWALL_MESSAGE_CODE: u8 = 0;
const PEER_INIT_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Clone)]
struct ConnectionContext {
    client_sender: UnboundedSender<ClientOperation>,
    client_context: Arc<ClientContext>,
    own_username: String,
}

struct PeerInitData {
    username: String,
    connection_type: ConnectionType,
    token: u32,
}

async fn read_peer_init_message(
    stream: &mut TcpStream,
    reader: &mut MessageReader,
) -> io::Result<Message> {
    let mut temp_buffer = [0u8; 1024];
    loop {
        let n = stream.read(&mut temp_buffer).await?;
        if n == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "Connection closed while reading peer init",
            ));
        }
        reader.push_bytes(&temp_buffer[..n]);

        if let Ok(Some(msg)) = reader.extract_message() {
            return Ok(msg);
        }
    }
}

fn parse_pierce_firewall_token(message: &mut Message) -> Option<PeerTransferToken> {
    message.set_pointer(4);
    let message_code = message.read_int8();

    if message_code != PIERCE_FIREWALL_MESSAGE_CODE {
        return None;
    }

    Some(PeerTransferToken(message.read_int32()))
}

fn parse_peer_init_message(mut message: Message) -> Option<PeerInitData> {
    message.set_pointer(4);
    let message_code = message.read_int8();

    if message_code != PEER_INIT_MESSAGE_CODE {
        return None;
    }

    Some(PeerInitData {
        username: message.read_string(),
        connection_type: message.read_string().parse().unwrap(),
        token: message.read_int32(),
    })
}

fn parse_token_from_buffer(buffer: &[u8], username: &str) -> Option<PeerTransferToken> {
    let token_bytes = buffer.get(0..4)?;
    let token = u32::from_le_bytes(token_bytes.try_into().unwrap_or_else(|_| {
        panic!(
            "[listener:{}] slice with incorrect length, can't extract transfer_token",
            username
        )
    }));
    Some(PeerTransferToken(token))
}

fn handle_peer_connection(
    peer: Peer,
    stream: TcpStream,
    reader: MessageReader,
    context: &ConnectionContext,
    _peer_ip: &str,
    _peer_port: u16,
) {
    match context.client_context.peer_registry.register_peer(peer.clone(), Some(stream), Some(reader)) {
        Ok(_) => (),
        Err(_e) => {
            error!(
                "Failed to spawn peer actor for {:?}: {:?}",
                peer.username, _e
            );
        }
    }
}

async fn handle_incoming_connection(stream: TcpStream, context: ConnectionContext) {
    let Ok(peer_addr) = stream.peer_addr() else {
        error!("[listener] failed to get peer address");
        return;
    };

    let peer_ip = peer_addr.ip().to_string();
    let peer_port = peer_addr.port();
    let mut stream = stream;
    let mut reader = MessageReader::new();

    let mut message = match tokio::time::timeout(
        PEER_INIT_TIMEOUT,
        read_peer_init_message(&mut stream, &mut reader),
    )
    .await
    {
        Ok(Ok(msg)) => msg,
        Ok(Err(_e)) => {
            error!("[listener:{peer_ip}:{peer_port}] Failed to read peer init message: {_e}");
            return;
        }
        Err(_) => {
            debug!("[listener:{peer_ip}:{peer_port}] Peer init timed out, dropping connection");
            return;
        }
    };

    // Check for PierceFireWall message (code 0)
    if let Some(token) = parse_pierce_firewall_token(&mut message) {
        debug!(
            "[listener:{peer_ip}:{peer_port}] PierceFireWall token: {}",
            token
        );

        // Query the worker for the download by token
        let (tx, rx) = oneshot::channel();
        let _ = context.client_sender.send(ClientOperation::QueryDownloadByToken(token, tx));
        let Some(download) = rx.await.ok().flatten() else {
            debug!(
                "[listener:{peer_ip}:{peer_port}] No download found for PierceFireWall token: {}",
                token
            );
            return;
        };

        let std_stream = stream.into_std().unwrap();
        spawn_direct_download(
            download,
            peer_ip,
            peer_port.into(),
            token.0,
            context.own_username.clone(),
            Some(std_stream),
            context.client_sender.clone(),
        );
        return;
    }

    let Some(init_data) = parse_peer_init_message(message) else {
        error!("[listener:{peer_ip}:{peer_port}] Invalid or unknown peer init message");
        return;
    };

    debug!(
        "[listener:{peer_ip}:{peer_port}] peerInit username: {} connection_type: {} token: {}",
        init_data.username, init_data.connection_type, init_data.token
    );

    let peer = Peer::new(
        init_data.username.clone(),
        init_data.connection_type.clone(),
        peer_ip.clone(),
        peer_port.into(),
        None,
        0,
        0,
        0,
    );

    match init_data.connection_type {
        ConnectionType::P => {
            handle_peer_connection(peer, stream, reader, &context, &peer_ip, peer_port)
        }

        ConnectionType::F => {
            // Pre-fetch the download token from the buffered data before spawn_blocking
            let buffer = reader.get_buffer();
            let Some(download_token) = parse_token_from_buffer(&buffer, &init_data.username) else {
                error!(
                    "[listener:{}:{}] No download token in buffer for F connection",
                    peer_ip, peer_port
                );
                return;
            };
            trace!(
                "[listener:{}] got transfer_token: {} from data chunk",
                init_data.username, download_token
            );

            // Query the worker for the download
            let (tx, rx) = oneshot::channel();
            let _ = context.client_sender.send(ClientOperation::QueryDownloadByToken(download_token, tx));
            let Some(download) = rx.await.ok().flatten() else {
                error!(
                    "[listener:{}:{}] No download found for file connection token: {}",
                    peer_ip, peer_port, download_token
                );
                return;
            };

            let std_stream = stream.into_std().unwrap();
            spawn_direct_download(
                download,
                peer.host,
                peer.port,
                init_data.token,
                context.own_username.clone(),
                Some(std_stream),
                context.client_sender.clone(),
            );
        }
        ConnectionType::D => {
            debug!(
                "[listener:{peer_ip}:{peer_port}] connection type is D, not supported yet, closing connection. "
            );
        }
    }
}

#[derive(thiserror::Error, Debug)]
pub enum ListenError {
    #[error("failed to bind listener to port {0}")]
    FailedToBindListener(#[from] io::Error),
}

pub struct Listen;

impl Listen {
    pub async fn start(
        port: u32,
        client_sender: UnboundedSender<ClientOperation>,
        client_context: Arc<ClientContext>,
        own_username: String,
    ) -> Result<(), ListenError> {
        info!("[listener] starting listener on port {port}");

        let listener = TcpListener::bind(format!("0.0.0.0:{port}"))
            .await
            .map_err(ListenError::FailedToBindListener)?;

        let context = ConnectionContext {
            client_sender,
            client_context,
            own_username,
        };

        loop {
            match listener.accept().await {
                Ok((stream, _addr)) => {
                    let context = context.clone();
                    tokio::spawn(async move {
                        handle_incoming_connection(stream, context).await;
                    });
                }
                Err(_e) => {
                    error!("[listener] Failed to accept connection: {}", _e);
                }
            }
        }
    }
}
