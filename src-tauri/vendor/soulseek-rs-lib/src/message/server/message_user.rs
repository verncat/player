use crate::actor::server_actor::{ServerSignal, UserMessage};
use crate::message::{Message, MessageHandler};
use tokio::sync::mpsc::UnboundedSender;

pub struct MessageUser;

fn decode(mut data: &[u8]) -> Option<UserMessage> {
    fn number(data: &mut &[u8]) -> Option<u32> {
        let value = u32::from_le_bytes(data.get(..4)?.try_into().ok()?);
        *data = &data[4..];
        Some(value)
    }
    fn string(data: &mut &[u8], max: usize) -> Option<String> {
        let size = number(data)? as usize;
        if size > max { return None; }
        let value = std::str::from_utf8(data.get(..size)?).ok()?.to_owned();
        *data = &data[size..];
        Some(value)
    }
    let id = number(&mut data)?;
    let timestamp = number(&mut data)?;
    let username = string(&mut data, 1024)?;
    let body = string(&mut data, 65536)?;
    let new_message = match data { [] => false, [0] => false, [1] => true, _ => return None };
    if username.is_empty() { return None; }
    Some(UserMessage::new(id, timestamp, username, body, new_message))
}

impl MessageHandler<ServerSignal> for MessageUser {
    fn get_code(&self) -> u32 { 22 }
    fn handle(&self, message: &mut Message, sender: UnboundedSender<ServerSignal>) {
        let bytes = message.get_data();
        if let Some(message) = bytes.get(8..).and_then(decode) {
            let _ = sender.send(ServerSignal::PrivateMessage(message));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn private_message_parser_rejects_truncated_and_oversized_packets() {
        let packet = Message::new().write_int32(42).write_int32(1700000000)
            .write_string("alice").write_string("Hello 🎵").write_bool(true).get_data();
        let parsed = decode(&packet).unwrap();
        assert_eq!(parsed.id, 42);
        assert_eq!(parsed.message, "Hello 🎵");
        for end in 0..packet.len()-1 { assert!(decode(&packet[..end]).is_none()); }
        assert!(!decode(&packet[..packet.len()-1]).unwrap().new_message);
        let mut invalid = packet.clone();
        invalid.push(0);
        assert!(decode(&invalid).is_none());
        assert!(decode(&Message::new().write_int32(1).write_int32(1).write_int32(u32::MAX).get_data()).is_none());
    }
}
