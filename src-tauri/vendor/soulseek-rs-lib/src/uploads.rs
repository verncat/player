//! Temporary, explicit per-recipient shares. No directory scanning or public search indexing.
use crate::{
    actor::peer_actor::PeerSignal,
    message::server::MessageFactory,
    message::{Message, MessageHandler},
    peer::ConnectionType,
};
use std::{
    collections::HashMap,
    fs::File,
    io,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
};

#[derive(Clone, Default)]
pub struct UploadShares(Arc<Mutex<HashMap<String, Share>>>);
struct Share {
    username: String,
    file: File,
    size: u64,
    expires: Instant,
    token: Option<u32>,
}
pub struct Upload {
    pub file: File,
    pub size: u64,
    pub username: String,
    pub token: u32,
}
impl UploadShares {
    pub fn add(&self, username: String, alias: String, file: File) -> io::Result<u64> {
        let meta = file.metadata()?;
        if !meta.is_file() || meta.len() == 0 {
            return Err(io::Error::other("Expected a nonempty audio file"));
        }
        let mut shares = self
            .0
            .lock()
            .map_err(|_| io::Error::other("Shares unavailable"))?;
        shares.retain(|_, share| share.expires > Instant::now());
        if shares.len() >= 32 || shares.contains_key(&alias) {
            return Err(io::Error::other("Too many active shares"));
        }
        let size = meta.len();
        shares.insert(
            alias,
            Share {
                username,
                file,
                size,
                expires: Instant::now() + Duration::from_secs(600),
                token: None,
            },
        );
        Ok(size)
    }
    pub fn remove(&self, alias: &str) {
        self.0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(alias);
    }
    pub fn clear(&self) {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).clear();
    }
    pub(crate) fn offer(&self, username: &str, alias: &str) -> Option<(u32, u64)> {
        use std::sync::atomic::{AtomicU32, Ordering};
        static NEXT: AtomicU32 = AtomicU32::new(1);
        let mut shares = self.0.lock().ok()?;
        let share = shares.get_mut(alias)?;
        if share.username != username || share.expires <= Instant::now() {
            return None;
        }
        let token = *share
            .token
            .get_or_insert_with(|| NEXT.fetch_add(1, Ordering::Relaxed));
        Some((token, share.size))
    }
    pub(crate) fn take(&self, username: &str, token: u32) -> Option<Upload> {
        let mut shares = self.0.lock().ok()?;
        let alias = shares
            .iter()
            .find(|(_, s)| {
                s.username == username && s.token == Some(token) && s.expires > Instant::now()
            })?
            .0
            .clone();
        let share = shares.remove(&alias)?;
        Some(Upload {
            file: share.file,
            size: share.size,
            username: username.to_owned(),
            token,
        })
    }
}

pub(crate) struct QueueUpload;
pub(crate) struct UploadDenied;
fn filename(message: &Message) -> Option<String> {
    let bytes = message.get_data();
    let length = u32::from_le_bytes(bytes.get(8..12)?.try_into().ok()?) as usize;
    if length == 0 || length > 1024 {
        return None;
    }
    Some(
        std::str::from_utf8(bytes.get(12..12 + length)?)
            .ok()?
            .to_owned(),
    )
}
impl MessageHandler<PeerSignal> for UploadDenied {
    fn get_code(&self) -> u32 {
        50
    }
    fn handle(
        &self,
        message: &mut Message,
        sender: tokio::sync::mpsc::UnboundedSender<PeerSignal>,
    ) {
        if let Some(alias) = filename(message) {
            let _ = sender.send(PeerSignal::UploadDenied(alias));
        }
    }
}
impl MessageHandler<PeerSignal> for QueueUpload {
    fn get_code(&self) -> u32 {
        43
    }
    fn handle(
        &self,
        message: &mut Message,
        sender: tokio::sync::mpsc::UnboundedSender<PeerSignal>,
    ) {
        // The dispatcher positions the cursor after the code. Parse bounded data without panics.
        if let Some(alias) = filename(message) {
            let _ = sender.send(PeerSignal::UploadRequested(alias));
        }
    }
}

pub(crate) async fn send(
    upload: Upload,
    host: String,
    port: u32,
    own_username: String,
) -> io::Result<()> {
    tokio::time::timeout(Duration::from_secs(300), async move {
        let mut stream = TcpStream::connect((
            host.as_str(),
            u16::try_from(port).map_err(|_| io::Error::other("Invalid port"))?,
        ))
        .await?;
        let init = MessageFactory::build_peer_init_message(&own_username, ConnectionType::F, 0);
        stream.write_all(&init.get_buffer()).await?;
        stream.write_all(&upload.token.to_le_bytes()).await?;
        let offset = stream.read_u64_le().await?;
        if offset > upload.size {
            return Err(io::Error::other("Invalid transfer offset"));
        }
        let mut file = tokio::fs::File::from_std(upload.file);
        use tokio::io::AsyncSeekExt;
        file.seek(std::io::SeekFrom::Start(offset)).await?;
        let mut source = file.take(upload.size - offset);
        let copied = tokio::io::copy(&mut source, &mut stream).await?;
        if copied != upload.size - offset {
            return Err(io::Error::other("Shared file changed during transfer"));
        }
        stream.shutdown().await
    })
    .await
    .map_err(|_| io::Error::other("Upload timed out"))?
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shares_are_recipient_scoped_and_consumed_once() {
        let file = File::open("Cargo.toml").unwrap();
        let shares = UploadShares::default();
        shares
            .add("alice".into(), "random\\track.flac".into(), file)
            .unwrap();
        assert!(shares.offer("eve", "random\\track.flac").is_none());
        let (token, _) = shares.offer("alice", "random\\track.flac").unwrap();
        assert!(shares.take("eve", token).is_none());
        assert!(shares.take("alice", token).is_some());
        assert!(shares.take("alice", token).is_none());
        shares
            .add(
                "alice".into(),
                "expired".into(),
                File::open("Cargo.toml").unwrap(),
            )
            .unwrap();
        shares.0.lock().unwrap().get_mut("expired").unwrap().expires =
            Instant::now() - Duration::from_secs(1);
        assert!(shares.offer("alice", "expired").is_none());
    }
    #[test]
    fn upload_filename_parser_rejects_truncated_and_oversized_packets() {
        let packet = Message::new()
            .write_int32(43)
            .write_string("request\\track.opus")
            .get_buffer();
        assert_eq!(
            filename(&Message::new_with_data(packet.clone())).as_deref(),
            Some("request\\track.opus")
        );
        for end in 0..packet.len() {
            assert!(filename(&Message::new_with_data(packet[..end].to_vec())).is_none());
        }
        let invalid = Message::new()
            .write_int32(43)
            .write_int32(u32::MAX)
            .get_buffer();
        assert!(filename(&Message::new_with_data(invalid)).is_none());
    }
}
