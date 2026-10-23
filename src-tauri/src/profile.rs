//! Local profiles and explicit device pairing. Key display requires a separate local command.
use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use serde::{Deserialize, Serialize};
use snow::resolvers::{CryptoResolver, DefaultResolver};
use std::fs;
use std::io::{Read, Write};
use std::net::IpAddr;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Manager};
use zeroize::Zeroizing;

const PATTERN: &str = "Noise_XX_25519_ChaChaPoly_BLAKE2s";
const LIFETIME: Duration = Duration::from_secs(180);
const LIMIT: usize = 4096;
type Result<T> = std::result::Result<T, String>;

fn noise_error(_: snow::Error) -> String {
    "Could not verify the secure pairing session. Try linking again.".into()
}

fn public_key(secret: &[u8]) -> Result<Vec<u8>> {
    if secret.len() != 32 {
        return Err("Invalid profile key length".into());
    }
    let mut dh = DefaultResolver
        .resolve_dh(&snow::params::DHChoice::Curve25519)
        .ok_or("Profile key algorithm is unavailable")?;
    dh.set(secret);
    Ok(dh.pubkey().to_vec())
}

#[derive(Clone, Serialize)]
pub struct ProfileInfo {
    pub user_id: String,
    pub public_key: String,
}

struct ProfileStore {
    path: Option<PathBuf>,
    secret: Zeroizing<Vec<u8>>,
}

impl ProfileStore {
    fn open(path: Option<PathBuf>) -> Result<Self> {
        let secret = match path.as_ref().map(fs::read) {
            Some(Ok(bytes)) => Zeroizing::new(bytes),
            Some(Err(error)) if error.kind() != std::io::ErrorKind::NotFound => {
                return Err(format!("Could not read the local profile: {error}"));
            }
            _ => Zeroizing::new(
                snow::Builder::new(PATTERN.parse().map_err(noise_error)?)
                    .generate_keypair()
                    .map_err(noise_error)?
                    .private,
            ),
        };
        public_key(&secret)?;
        let store = Self { path, secret };
        // Also enforce private permissions on existing profile files.
        store.persist(&store.secret)?;
        Ok(store)
    }

    fn persist(&self, secret: &[u8]) -> Result<()> {
        let Some(path) = &self.path else {
            return Ok(());
        };
        let parent = path.parent().ok_or("Invalid profile storage path")?;
        fs::create_dir_all(parent).map_err(|e| format!("Could not create profile storage: {e}"))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(parent, fs::Permissions::from_mode(0o700))
                .map_err(|e| format!("Could not protect profile storage: {e}"))?;
        }
        // tempfile creates private files and persist atomically replaces on Windows too.
        let mut file = tempfile::NamedTempFile::new_in(parent)
            .map_err(|e| format!("Could not save the local profile: {e}"))?;
        file.write_all(secret)
            .and_then(|_| file.as_file().sync_all())
            .map_err(|e| format!("Could not save the local profile: {e}"))?;
        file.persist(path)
            .map_err(|e| format!("Could not replace the local profile: {}", e.error))?;
        Ok(())
    }

    fn info(&self) -> ProfileInfo {
        let public = public_key(&self.secret).expect("validated profile key");
        ProfileInfo {
            user_id: uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_OID, &public).to_string(),
            public_key: BASE64.encode(public),
        }
    }

    fn import(&mut self, secret: Zeroizing<Vec<u8>>, expected_public: &[u8]) -> Result<()> {
        if public_key(&secret)? != expected_public {
            return Err("The received profile does not match the verified device".into());
        }
        self.persist(&secret)?;
        self.secret = secret;
        Ok(())
    }
}

#[derive(Clone, Serialize, Deserialize)]
struct DeviceLabel {
    device_id: String,
    device_name: String,
}

impl DeviceLabel {
    fn local(app: &AppHandle) -> Result<Self> {
        let settings = app
            .state::<crate::library::LibraryState>()
            .get_device_settings()
            .map_err(|_| "Could not read this device's identity")?;
        Ok(Self {
            device_id: settings.device_id,
            device_name: settings.device_name,
        })
    }

    fn validate(&self, local: &Self) -> Result<()> {
        uuid::Uuid::parse_str(&self.device_id).map_err(|_| "Invalid device identity")?;
        if self.device_id == local.device_id {
            return Err(
                "These installations have the same device ID. Use a separate app database.".into(),
            );
        }
        if self.device_name.len() > 256 {
            return Err("Device name is too long".into());
        }
        Ok(())
    }
}

// Only Noise handshake messages and encrypted transport messages cross the HTTP API.
#[derive(Serialize, Deserialize)]
struct WireMessage {
    session_id: String,
    message: String,
}

enum Channel {
    Handshake(Box<snow::HandshakeState>),
    Transport(Box<snow::TransportState>),
}

struct Session {
    id: String,
    peer: DeviceLabel,
    channel: Option<Channel>,
    code: Option<String>,
    confirmed: bool,
    remote_url: Option<String>, // Only the joining device polls the source.
    remote_public: Vec<u8>,
}

#[derive(Default)]
struct Pairing {
    phase: &'static str,
    expires: Option<Instant>,
    session: Option<Session>,
    error: Option<String>,
    attempts: u8,
}

#[derive(Serialize)]
pub struct PairingView {
    phase: &'static str,
    role: Option<&'static str>,
    peer_name: Option<String>,
    code: Option<String>,
    confirmed: bool,
    expires_in: u64,
    error: Option<String>,
}

impl Pairing {
    fn expire(&mut self) {
        if self.expires.is_some_and(|end| Instant::now() >= end) {
            self.session = None;
            self.expires = None;
            if self.phase != "complete" {
                self.phase = "expired";
            }
        }
    }

    fn view(&mut self) -> PairingView {
        self.expire();
        PairingView {
            phase: if self.phase.is_empty() {
                "idle"
            } else {
                self.phase
            },
            role: self.session.as_ref().map(|s| {
                if s.remote_url.is_some() {
                    "join"
                } else {
                    "share"
                }
            }),
            peer_name: self.session.as_ref().map(|s| s.peer.device_name.clone()),
            code: self.session.as_ref().and_then(|s| s.code.clone()),
            confirmed: self.session.as_ref().is_some_and(|s| s.confirmed),
            expires_in: self.expires.map_or(0, |end| {
                end.saturating_duration_since(Instant::now()).as_secs()
            }),
            error: self.error.clone(),
        }
    }

    fn check_idle(&mut self) -> Result<()> {
        self.expire();
        if self.expires.is_some() {
            return Err("Finish or cancel the current pairing first".into());
        }
        Ok(())
    }

    fn session(&mut self, id: &str) -> Result<&mut Session> {
        self.expire();
        self.session
            .as_mut()
            .filter(|s| s.id == id)
            .ok_or_else(|| "Pairing was cancelled or expired. Start again on both devices.".into())
    }

    fn fail(&mut self, error: String) {
        self.session = None;
        self.expires = None;
        self.phase = "error";
        self.error = Some(error);
    }
}

pub struct LocalProfileState {
    store: Mutex<ProfileStore>,
    pairing: Mutex<Pairing>,
}

impl LocalProfileState {
    pub fn open(path: Option<PathBuf>) -> Result<Self> {
        Ok(Self {
            store: Mutex::new(ProfileStore::open(path)?),
            pairing: Mutex::new(Pairing::default()),
        })
    }

    pub fn info(&self) -> ProfileInfo {
        self.store.lock().unwrap().info()
    }

    fn allow(&self) -> Result<PairingView> {
        let mut pairing = self.pairing.lock().unwrap();
        pairing.check_idle()?;
        *pairing = Pairing {
            phase: "listening",
            expires: Some(Instant::now() + LIFETIME),
            ..Default::default()
        };
        Ok(pairing.view())
    }

    fn accept_start(&self, local: DeviceLabel, wire: WireMessage) -> Result<WireMessage> {
        let mut pairing = self.pairing.lock().unwrap();
        pairing.expire();
        if pairing.phase != "listening" || pairing.expires.is_none() || pairing.session.is_some() {
            return Err("On the other device, open Devices and choose Allow linking first.".into());
        }
        pairing.attempts += 1;
        if pairing.attempts > 5 {
            pairing.fail("Too many linking attempts. Start a new session on both devices.".into());
            return Err("Linking is closed. Start a new session on both devices.".into());
        }
        uuid::Uuid::parse_str(&wire.session_id).map_err(|_| "Invalid pairing session")?;
        let store = self.store.lock().unwrap();
        let mut handshake = handshake(&store.secret, &wire.session_id, false)?;
        let peer: DeviceLabel =
            serde_json::from_slice(&read_handshake(&mut handshake, &wire.message)?)
                .map_err(|_| "Invalid pairing device")?;
        peer.validate(&local)?;
        let reply = write_handshake(&mut handshake, &serde_json::to_vec(&local).unwrap())?;
        pairing.session = Some(Session {
            id: wire.session_id.clone(),
            peer,
            channel: Some(Channel::Handshake(Box::new(handshake))),
            code: None,
            confirmed: false,
            remote_url: None,
            remote_public: Vec::new(),
        });
        pairing.phase = "connecting";
        pairing.expires = Some(Instant::now() + LIFETIME);
        Ok(WireMessage {
            session_id: wire.session_id,
            message: reply,
        })
    }

    fn accept_finish(&self, wire: WireMessage) -> Result<()> {
        let mut pairing = self.pairing.lock().unwrap();
        let session = pairing.session(&wire.session_id)?;
        if session.remote_url.is_some() {
            return Err("Invalid pairing direction".into());
        }
        let Some(Channel::Handshake(mut handshake)) = session.channel.take() else {
            return Err("Pairing handshake has already finished".into());
        };
        read_handshake(&mut handshake, &wire.message)?;
        session.code = Some(verification_code(&handshake));
        session.remote_public = handshake
            .get_remote_static()
            .ok_or("Missing peer key")?
            .to_vec();
        session.channel = Some(Channel::Transport(Box::new(
            handshake.into_transport_mode().map_err(noise_error)?,
        )));
        pairing.phase = "verify";
        Ok(())
    }

    fn accept_exchange(&self, wire: WireMessage) -> Result<WireMessage> {
        let mut pairing = self.pairing.lock().unwrap();
        let session = pairing.session(&wire.session_id)?;
        if session.remote_url.is_some() {
            return Err("Invalid pairing direction".into());
        }
        let message = decrypt(session, &wire.message)?;
        let mut reply = Zeroizing::new(Vec::new());
        let mut complete = false;
        let mut cancel = false;
        match message.as_slice() {
            b"status" => reply.extend_from_slice(b"wait"),
            b"confirm" if !session.confirmed => reply.extend_from_slice(b"wait"),
            b"confirm" => {
                reply.extend_from_slice(b"key:");
                reply.extend_from_slice(&self.store.lock().unwrap().secret);
                complete = true;
            }
            b"cancel" => {
                reply.extend_from_slice(b"cancelled");
                cancel = true;
            }
            _ => return Err("Invalid pairing message".into()),
        }
        let message = encrypt(session, &reply)?;
        if complete {
            pairing.phase = "complete";
        }
        if cancel {
            *pairing = Pairing {
                phase: "cancelled",
                ..Default::default()
            };
        }
        Ok(WireMessage {
            session_id: wire.session_id,
            message,
        })
    }

    fn join(
        &self,
        local: DeviceLabel,
        url: String,
        expected_public: String,
        expected_device: String,
    ) -> Result<PairingView> {
        let mut pairing = self.pairing.lock().unwrap();
        pairing.check_idle()?;
        let remote_public = BASE64
            .decode(expected_public)
            .map_err(|_| "Invalid profile public key")?;
        if remote_public.len() != 32 {
            return Err("Invalid profile public key".into());
        }
        if self.info().public_key == BASE64.encode(&remote_public) {
            return Err("This device already uses that profile".into());
        }
        let id = uuid::Uuid::new_v4().to_string();
        let store = self.store.lock().unwrap();
        let mut handshake = handshake(&store.secret, &id, true)?;
        drop(store);
        let message = write_handshake(&mut handshake, &serde_json::to_vec(&local).unwrap())?;
        let reply: WireMessage = post(
            &url,
            "start",
            &WireMessage {
                session_id: id.clone(),
                message,
            },
        )?;
        if reply.session_id != id {
            return Err("Pairing session changed".into());
        }
        let peer: DeviceLabel =
            serde_json::from_slice(&read_handshake(&mut handshake, &reply.message)?)
                .map_err(|_| "Invalid pairing device")?;
        peer.validate(&local)?;
        if peer.device_id != expected_device
            || handshake.get_remote_static() != Some(remote_public.as_slice())
        {
            return Err("The device identity changed. Refresh Devices and try again.".into());
        }
        let message = write_handshake(&mut handshake, &[])?;
        let code = verification_code(&handshake);
        let _: serde_json::Value = post(
            &url,
            "finish",
            &WireMessage {
                session_id: id.clone(),
                message,
            },
        )?;
        let channel = handshake.into_transport_mode().map_err(noise_error)?;
        *pairing = Pairing {
            phase: "verify",
            expires: Some(Instant::now() + LIFETIME),
            error: None,
            attempts: 0,
            session: Some(Session {
                id,
                peer,
                channel: Some(Channel::Transport(Box::new(channel))),
                code: Some(code),
                confirmed: false,
                remote_url: Some(url),
                remote_public,
            }),
        };
        Ok(pairing.view())
    }

    fn confirm(&self) -> Result<PairingView> {
        let mut pairing = self.pairing.lock().unwrap();
        pairing.expire();
        let session = pairing.session.as_mut().ok_or("No active pairing")?;
        if session.code.is_none() {
            return Err("The pairing code is not ready yet".into());
        }
        session.confirmed = true;
        pairing.phase = "waiting";
        Ok(pairing.view())
    }

    fn poll(&self) -> PairingView {
        let mut pairing = self.pairing.lock().unwrap();
        pairing.expire();
        if !matches!(pairing.phase, "verify" | "waiting") {
            return pairing.view();
        }
        let result = (|| -> Result<bool> {
            let Some(session) = pairing.session.as_mut() else {
                return Ok(false);
            };
            let Some(url) = session.remote_url.clone() else {
                return Ok(false);
            };
            let payload: &[u8] = if session.confirmed {
                b"confirm"
            } else {
                b"status"
            };
            let wire = WireMessage {
                session_id: session.id.clone(),
                message: encrypt(session, payload)?,
            };
            let reply: WireMessage = post(&url, "exchange", &wire)?;
            if reply.session_id != session.id {
                return Err("Pairing session changed".into());
            }
            let message = decrypt(session, &reply.message)?;
            if message.as_slice() == b"wait" {
                return Ok(false);
            }
            if !session.confirmed {
                return Err("Unexpected profile transfer before confirmation".into());
            }
            let secret = message
                .strip_prefix(b"key:")
                .ok_or("Invalid profile transfer")?;
            self.store
                .lock()
                .unwrap()
                .import(Zeroizing::new(secret.to_vec()), &session.remote_public)?;
            Ok(true)
        })();
        match result {
            Ok(true) => {
                pairing.phase = "complete";
                pairing.expires = None;
                pairing.session = None;
            }
            Ok(false) => {}
            Err(error) => pairing.fail(error),
        }
        pairing.view()
    }

    fn cancel(&self) {
        let mut pairing = self.pairing.lock().unwrap();
        if let Some(session) = pairing.session.as_mut() {
            if let Some(url) = session.remote_url.clone() {
                if let Ok(message) = encrypt(session, b"cancel") {
                    let _: Result<WireMessage> = post(
                        &url,
                        "exchange",
                        &WireMessage {
                            session_id: session.id.clone(),
                            message,
                        },
                    );
                }
            }
        }
        *pairing = Pairing {
            phase: "cancelled",
            ..Default::default()
        };
    }
}

fn handshake(secret: &[u8], id: &str, initiator: bool) -> Result<snow::HandshakeState> {
    let prologue = format!("Player local profile pairing v1:{id}");
    let builder = snow::Builder::new(PATTERN.parse().map_err(noise_error)?)
        .local_private_key(secret)
        .map_err(noise_error)?
        .prologue(prologue.as_bytes())
        .map_err(noise_error)?;
    if initiator {
        builder.build_initiator()
    } else {
        builder.build_responder()
    }
    .map_err(noise_error)
}

fn read_handshake(handshake: &mut snow::HandshakeState, message: &str) -> Result<Vec<u8>> {
    let bytes = BASE64
        .decode(message)
        .map_err(|_| "Invalid handshake encoding")?;
    let mut buffer = vec![0u8; LIMIT];
    let len = handshake
        .read_message(&bytes, &mut buffer)
        .map_err(noise_error)?;
    buffer.truncate(len);
    Ok(buffer)
}

fn write_handshake(handshake: &mut snow::HandshakeState, payload: &[u8]) -> Result<String> {
    let mut buffer = vec![0u8; LIMIT];
    let len = handshake
        .write_message(payload, &mut buffer)
        .map_err(noise_error)?;
    Ok(BASE64.encode(&buffer[..len]))
}

fn verification_code(handshake: &snow::HandshakeState) -> String {
    // 48 transcript bits; both screens must compare all three groups.
    handshake.get_handshake_hash()[..6]
        .chunks(2)
        .map(|bytes| format!("{:02X}{:02X}", bytes[0], bytes[1]))
        .collect::<Vec<_>>()
        .join(" ")
}

fn encrypt(session: &mut Session, payload: &[u8]) -> Result<String> {
    let Some(Channel::Transport(transport)) = session.channel.as_mut() else {
        return Err("Pairing is not ready".into());
    };
    let mut buffer = vec![0u8; LIMIT];
    let len = transport
        .write_message(payload, &mut buffer)
        .map_err(noise_error)?;
    Ok(BASE64.encode(&buffer[..len]))
}

fn decrypt(session: &mut Session, message: &str) -> Result<Zeroizing<Vec<u8>>> {
    let Some(Channel::Transport(transport)) = session.channel.as_mut() else {
        return Err("Pairing is not ready".into());
    };
    let bytes = BASE64
        .decode(message)
        .map_err(|_| "Invalid encrypted message")?;
    let mut buffer = Zeroizing::new(vec![0u8; LIMIT]);
    let len = transport
        .read_message(&bytes, &mut buffer)
        .map_err(noise_error)?;
    buffer.truncate(len);
    Ok(buffer)
}

fn post<T: serde::de::DeserializeOwned>(
    url: &str,
    action: &str,
    message: &WireMessage,
) -> Result<T> {
    let response = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(4))
        .redirect(reqwest::redirect::Policy::none())
        .no_proxy()
        .build()
        .map_err(|_| "Could not connect to the other device")?
        .post(format!("{url}/profile-pairing/{action}"))
        .json(message)
        .send()
        .map_err(|_| {
            "Could not reach the other device. Check the connection and try linking again."
        })?;
    let success = response.status().is_success();
    let mut bytes = Vec::new();
    response
        .take(LIMIT as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "Could not read the pairing response")?;
    if bytes.len() > LIMIT {
        return Err("Pairing response is too large".into());
    }
    if !success {
        #[derive(Deserialize)]
        struct Failure {
            error: String,
        }
        return Err(serde_json::from_slice::<Failure>(&bytes)
            .map(|f| f.error)
            .unwrap_or_else(|_| "The other device declined or cancelled pairing".into()));
    }
    serde_json::from_slice(&bytes).map_err(|_| "Invalid pairing response".into())
}

pub fn serve(mut request: tiny_http::Request, app: &AppHandle) {
    let result = (|| -> Result<serde_json::Value> {
        if request.method() != &tiny_http::Method::Post {
            return Err("Pairing requires POST".into());
        }
        let content_type = request
            .headers()
            .iter()
            .find(|h| h.field.equiv("Content-Type"))
            .map(|h| h.value.as_str())
            .unwrap_or("");
        if content_type.split(';').next().map(str::trim) != Some("application/json") {
            return Err("Pairing requires JSON".into());
        }
        let mut bytes = Vec::new();
        request
            .as_reader()
            .take(LIMIT as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| "Could not read pairing request")?;
        if bytes.len() > LIMIT {
            return Err("Pairing request is too large".into());
        }
        let wire: WireMessage =
            serde_json::from_slice(&bytes).map_err(|_| "Invalid pairing request")?;
        let state = app.state::<LocalProfileState>();
        match request.url() {
            "/profile-pairing/start" => Ok(serde_json::to_value(
                state.accept_start(DeviceLabel::local(app)?, wire)?,
            )
            .unwrap()),
            "/profile-pairing/finish" => {
                state.accept_finish(wire)?;
                Ok(serde_json::json!({"ok": true}))
            }
            "/profile-pairing/exchange" => {
                Ok(serde_json::to_value(state.accept_exchange(wire)?).unwrap())
            }
            _ => Err("Unknown pairing endpoint".into()),
        }
    })();
    let (status, body) = match result {
        Ok(value) => (200, value),
        Err(error) => (400, serde_json::json!({"error": error})),
    };
    let _ = request.respond(
        tiny_http::Response::from_string(body.to_string())
            .with_status_code(status)
            .with_header(
                tiny_http::Header::from_bytes("Content-Type", "application/json").unwrap(),
            ),
    );
}

#[tauri::command]
pub fn local_profile_info(state: tauri::State<'_, LocalProfileState>) -> ProfileInfo {
    state.info()
}

/// Called only by the explicit Show action in Devices & Sync settings.
#[tauri::command]
pub fn local_profile_private_key(state: tauri::State<'_, LocalProfileState>) -> String {
    BASE64.encode(&*state.store.lock().unwrap().secret)
}

#[tauri::command]
pub async fn profile_pairing_allow(app: AppHandle) -> Result<PairingView> {
    tauri::async_runtime::spawn_blocking(move || app.state::<LocalProfileState>().allow())
        .await
        .map_err(|_| "Pairing task failed")?
}

#[tauri::command]
pub async fn profile_pairing_confirm(app: AppHandle) -> Result<PairingView> {
    tauri::async_runtime::spawn_blocking(move || app.state::<LocalProfileState>().confirm())
        .await
        .map_err(|_| "Pairing task failed")?
}

#[tauri::command]
pub async fn profile_pairing_join(
    app: AppHandle,
    host: String,
    port: u16,
    public_key: String,
    device_id: String,
) -> Result<PairingView> {
    // Only connect to the discovered IP and port; never follow a redirect with key material.
    let ip: IpAddr = host
        .parse()
        .map_err(|_| "This device has no reachable IP address yet")?;
    if ip.is_unspecified() || ip.is_multicast() || port == 0 {
        return Err("Invalid device address".into());
    }
    let host = if ip.is_ipv6() {
        format!("[{ip}]")
    } else {
        ip.to_string()
    };
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<LocalProfileState>().join(
            DeviceLabel::local(&app)?,
            format!("http://{host}:{port}"),
            public_key,
            device_id,
        )
    })
    .await
    .map_err(|_| "Pairing task failed")?
}

#[tauri::command]
pub async fn profile_pairing_poll(app: AppHandle) -> Result<PairingView> {
    tauri::async_runtime::spawn_blocking(move || app.state::<LocalProfileState>().poll())
        .await
        .map_err(|_| "Pairing task failed".into())
}

#[tauri::command]
pub async fn profile_pairing_cancel(app: AppHandle) -> Result<()> {
    tauri::async_runtime::spawn_blocking(move || app.state::<LocalProfileState>().cancel())
        .await
        .map_err(|_| "Pairing task failed".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    };

    fn label(name: &str) -> DeviceLabel {
        DeviceLabel {
            device_id: uuid::Uuid::new_v4().to_string(),
            device_name: name.into(),
        }
    }

    fn memory() -> LocalProfileState {
        LocalProfileState::open(None).unwrap()
    }

    fn pair(source: &LocalProfileState, client: &LocalProfileState) -> Session {
        source.allow().unwrap();
        let id = uuid::Uuid::new_v4().to_string();
        let client_label = label("Phone");
        let mut initiator = handshake(&client.store.lock().unwrap().secret, &id, true).unwrap();
        let message =
            write_handshake(&mut initiator, &serde_json::to_vec(&client_label).unwrap()).unwrap();
        let reply = source
            .accept_start(
                label("Desktop"),
                WireMessage {
                    session_id: id.clone(),
                    message,
                },
            )
            .unwrap();
        let peer: DeviceLabel =
            serde_json::from_slice(&read_handshake(&mut initiator, &reply.message).unwrap())
                .unwrap();
        let public = initiator.get_remote_static().unwrap().to_vec();
        assert_eq!(BASE64.encode(&public), source.info().public_key);
        let message = write_handshake(&mut initiator, &[]).unwrap();
        source
            .accept_finish(WireMessage {
                session_id: id.clone(),
                message,
            })
            .unwrap();
        let code = verification_code(&initiator);
        assert_eq!(source.poll().code.as_deref(), Some(code.as_str()));
        Session {
            id,
            peer,
            channel: Some(Channel::Transport(Box::new(
                initiator.into_transport_mode().unwrap(),
            ))),
            code: Some(code),
            confirmed: false,
            remote_url: Some("unused".into()),
            remote_public: public,
        }
    }

    fn exchange(
        source: &LocalProfileState,
        client: &mut Session,
        payload: &[u8],
    ) -> Zeroizing<Vec<u8>> {
        let message = encrypt(client, payload).unwrap();
        let reply = source
            .accept_exchange(WireMessage {
                session_id: client.id.clone(),
                message,
            })
            .unwrap();
        decrypt(client, &reply.message).unwrap()
    }

    #[test]
    fn profile_is_persistent_private_and_invalid_storage_is_not_overwritten() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("identity/profile.key");
        let original = LocalProfileState::open(Some(path.clone())).unwrap();
        let info = original.info();
        assert_eq!(
            uuid::Uuid::parse_str(&info.user_id)
                .unwrap()
                .get_version_num(),
            5
        );
        assert_eq!(fs::read(&path).unwrap().len(), 32);
        assert_eq!(
            LocalProfileState::open(Some(path.clone()))
                .unwrap()
                .info()
                .user_id,
            info.user_id
        );
        assert_ne!(memory().info().user_id, info.user_id);
        let public_json = serde_json::to_value(info).unwrap();
        assert_eq!(public_json.as_object().unwrap().len(), 2);
        assert!(public_json.get("secret").is_none());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
            assert_eq!(
                fs::metadata(path.parent().unwrap())
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o700
            );
        }
        fs::write(&path, b"damaged").unwrap();
        assert!(LocalProfileState::open(Some(path.clone())).is_err());
        assert_eq!(fs::read(path).unwrap(), b"damaged");
    }

    #[test]
    fn profile_transfer_requires_both_confirmations_and_preserves_source() {
        let source = memory();
        let client = memory();
        let original_source = source.info();
        let original_client = client.info();
        let mut session = pair(&source, &client);
        assert_eq!(
            exchange(&source, &mut session, b"status").as_slice(),
            b"wait"
        );
        assert_eq!(
            exchange(&source, &mut session, b"confirm").as_slice(),
            b"wait"
        );
        source.confirm().unwrap();
        // Confirming the source alone must still never export the key.
        assert_eq!(
            exchange(&source, &mut session, b"status").as_slice(),
            b"wait"
        );
        let received = exchange(&source, &mut session, b"confirm");
        let key = received.strip_prefix(b"key:").unwrap();
        client
            .store
            .lock()
            .unwrap()
            .import(Zeroizing::new(key.to_vec()), &session.remote_public)
            .unwrap();
        assert_ne!(original_client.user_id, client.info().user_id);
        assert_eq!(original_source.user_id, client.info().user_id);
        assert_eq!(original_source.public_key, client.info().public_key);
        assert_eq!(original_source.user_id, source.info().user_id);
        // Any device with the shared key can link another device without a primary.
        client.allow().unwrap();
        assert_eq!(client.poll().phase, "listening");
    }

    #[test]
    fn replay_and_modified_ciphertext_cannot_request_a_profile() {
        let source = memory();
        let client = memory();
        let mut session = pair(&source, &client);
        source.confirm().unwrap();
        let message = encrypt(&mut session, b"confirm").unwrap();
        let mut corrupt = BASE64.decode(&message).unwrap();
        corrupt[0] ^= 1;
        assert!(source
            .accept_exchange(WireMessage {
                session_id: session.id.clone(),
                message: BASE64.encode(corrupt)
            })
            .is_err());
        let reply = source
            .accept_exchange(WireMessage {
                session_id: session.id.clone(),
                message: message.clone(),
            })
            .unwrap();
        let ciphertext = BASE64.decode(&reply.message).unwrap();
        assert!(!ciphertext
            .windows(32)
            .any(|window| window == source.store.lock().unwrap().secret.as_slice()));
        assert!(source
            .accept_exchange(WireMessage {
                session_id: session.id.clone(),
                message
            })
            .is_err());
        assert!(decrypt(&mut session, &reply.message)
            .unwrap()
            .starts_with(b"key:"));
    }

    #[test]
    fn cancellation_expiry_and_wrong_keys_do_not_replace_the_profile() {
        let source = memory();
        let client = memory();
        let original = client.info().public_key;
        let mut session = pair(&source, &client);
        assert!(source.allow().is_err());
        source.cancel();
        let message = encrypt(&mut session, b"confirm").unwrap();
        assert!(source
            .accept_exchange(WireMessage {
                session_id: session.id,
                message
            })
            .is_err());
        let mut session = pair(&source, &client);
        source.pairing.lock().unwrap().expires = Some(Instant::now() - Duration::from_secs(1));
        let message = encrypt(&mut session, b"confirm").unwrap();
        assert!(source
            .accept_exchange(WireMessage {
                session_id: session.id,
                message
            })
            .is_err());
        assert_eq!(source.poll().phase, "expired");
        assert!(client
            .store
            .lock()
            .unwrap()
            .import(Zeroizing::new(vec![0; 32]), &[0; 32])
            .is_err());
        assert_eq!(client.info().public_key, original);
    }

    #[test]
    fn session_id_is_bound_into_the_handshake_and_requests_require_an_open_gate() {
        let source = memory();
        let client = memory();
        let id = uuid::Uuid::new_v4().to_string();
        let mut initiator = handshake(&client.store.lock().unwrap().secret, &id, true).unwrap();
        let message = write_handshake(
            &mut initiator,
            &serde_json::to_vec(&label("Phone")).unwrap(),
        )
        .unwrap();
        assert!(source
            .accept_start(
                label("Desktop"),
                WireMessage {
                    session_id: id.clone(),
                    message: message.clone()
                }
            )
            .is_err());
        source.allow().unwrap();
        let reply = source
            .accept_start(
                label("Desktop"),
                WireMessage {
                    session_id: uuid::Uuid::new_v4().to_string(),
                    message,
                },
            )
            .unwrap();
        assert!(read_handshake(&mut initiator, &reply.message).is_err());
    }

    #[test]
    fn repeated_invalid_starts_close_the_pairing_window() {
        let source = memory();
        source.allow().unwrap();
        for _ in 0..6 {
            assert!(source
                .accept_start(
                    label("Desktop"),
                    WireMessage {
                        session_id: uuid::Uuid::new_v4().to_string(),
                        message: "invalid".into(),
                    }
                )
                .is_err());
        }
        assert_eq!(source.poll().phase, "error");
        assert!(source.confirm().is_err());
        assert!(source.pairing.lock().unwrap().session.is_none());
        source.allow().unwrap();
        assert_eq!(source.poll().phase, "listening");
    }

    // Exercise the real client HTTP coordinator, including polling and atomic key replacement.
    struct PeerServer {
        state: Arc<LocalProfileState>,
        device: DeviceLabel,
        url: String,
        stop: Arc<AtomicBool>,
        thread: Option<std::thread::JoinHandle<()>>,
    }
    impl PeerServer {
        fn start() -> Self {
            let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
            let url = format!("http://{}", server.server_addr().to_ip().unwrap());
            let state = Arc::new(memory());
            let device = label("Desktop");
            let stop = Arc::new(AtomicBool::new(false));
            let worker_stop = stop.clone();
            let worker_state = state.clone();
            let worker_device = device.clone();
            let thread = std::thread::spawn(move || {
                while !worker_stop.load(Ordering::Relaxed) {
                    let Ok(Some(mut request)) = server.recv_timeout(Duration::from_millis(50))
                    else {
                        continue;
                    };
                    let mut bytes = Vec::new();
                    request.as_reader().read_to_end(&mut bytes).unwrap();
                    let wire = serde_json::from_slice(&bytes).unwrap();
                    let result = match request.url() {
                        "/profile-pairing/start" => worker_state
                            .accept_start(worker_device.clone(), wire)
                            .map(|r| serde_json::to_value(r).unwrap()),
                        "/profile-pairing/finish" => worker_state
                            .accept_finish(wire)
                            .map(|_| serde_json::json!({"ok": true})),
                        "/profile-pairing/exchange" => worker_state
                            .accept_exchange(wire)
                            .map(|r| serde_json::to_value(r).unwrap()),
                        _ => Err("Unknown endpoint".into()),
                    };
                    let (status, value) = match result {
                        Ok(value) => (200, value),
                        Err(error) => (400, serde_json::json!({"error":error})),
                    };
                    request
                        .respond(
                            tiny_http::Response::from_string(value.to_string())
                                .with_status_code(status),
                        )
                        .unwrap();
                }
            });
            Self {
                state,
                device,
                url,
                stop,
                thread: Some(thread),
            }
        }
    }
    impl Drop for PeerServer {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::Relaxed);
            self.thread.take().unwrap().join().unwrap();
        }
    }

    #[test]
    fn http_pairing_copies_the_profile_after_both_devices_confirm() {
        let peer = PeerServer::start();
        peer.state.allow().unwrap();
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("profile.key");
        let client = LocalProfileState::open(Some(path.clone())).unwrap();
        let original_id = client.info().user_id;
        let view = client
            .join(
                label("Phone"),
                peer.url.clone(),
                peer.state.info().public_key,
                peer.device.device_id.clone(),
            )
            .unwrap();
        assert_eq!(view.code, peer.state.poll().code);
        assert_eq!(client.poll().phase, "verify");
        client.confirm().unwrap();
        assert_eq!(client.poll().phase, "waiting");
        assert_eq!(client.info().user_id, original_id);
        peer.state.confirm().unwrap();
        assert_eq!(client.poll().phase, "complete");
        assert_eq!(client.info().user_id, peer.state.info().user_id);
        assert_eq!(
            LocalProfileState::open(Some(path)).unwrap().info().user_id,
            peer.state.info().user_id
        );
        assert!(client
            .join(
                label("Phone"),
                peer.url.clone(),
                peer.state.info().public_key,
                peer.device.device_id.clone()
            )
            .is_err());
    }

    #[test]
    fn joining_a_device_with_a_changed_identity_fails_and_cancel_propagates() {
        let peer = PeerServer::start();
        let client = memory();
        peer.state.allow().unwrap();
        assert!(client
            .join(
                label("Phone"),
                peer.url.clone(),
                peer.state.info().public_key,
                uuid::Uuid::new_v4().to_string()
            )
            .is_err());
        peer.state.cancel();
        peer.state.allow().unwrap();
        client
            .join(
                label("Phone"),
                peer.url.clone(),
                peer.state.info().public_key,
                peer.device.device_id.clone(),
            )
            .unwrap();
        client.cancel();
        assert_eq!(peer.state.poll().phase, "cancelled");
        assert_ne!(client.info().user_id, peer.state.info().user_id);
    }
}
