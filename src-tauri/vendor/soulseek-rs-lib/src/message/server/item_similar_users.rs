use crate::actor::server_actor::ServerSignal;
use crate::message::{Message, MessageHandler};
use tokio::sync::mpsc::UnboundedSender;

pub struct ItemSimilarUsersHandler;

// Bound strings/counts before allocating and reject truncated packets.
fn decode(mut data: &[u8]) -> Option<(String, Vec<String>)> {
    fn number(data: &mut &[u8]) -> Option<usize> {
        let value = u32::from_le_bytes(data.get(..4)?.try_into().ok()?) as usize;
        *data = &data[4..];
        Some(value)
    }
    fn string(data: &mut &[u8], max: usize) -> Option<String> {
        let length = number(data)?;
        if length > max {
            return None;
        }
        let text = std::str::from_utf8(data.get(..length)?).ok()?.to_owned();
        *data = &data[length..];
        Some(text)
    }
    let item = string(&mut data, 256)?;
    let count = number(&mut data)?;
    if count > 1000 {
        return None;
    }
    let mut users = Vec::with_capacity(count);
    for _ in 0..count {
        users.push(string(&mut data, 1024)?);
    }
    if !data.is_empty() {
        return None;
    }
    users.sort();
    users.dedup();
    Some((item, users))
}

impl MessageHandler<ServerSignal> for ItemSimilarUsersHandler {
    fn get_code(&self) -> u32 {
        112
    }
    fn handle(&self, message: &mut Message, sender: UnboundedSender<ServerSignal>) {
        let data = message.get_data();
        if let Some((item, users)) = data.get(8..).and_then(decode) {
            let _ = sender.send(ServerSignal::InterestUsers { item, users });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bounded_response_parser() {
        let body = Message::new()
            .write_string("adhdplayer-v1-test")
            .write_int32(3)
            .write_string("bob")
            .write_string("alice")
            .write_string("bob")
            .get_data();
        assert_eq!(
            decode(&body),
            Some((
                "adhdplayer-v1-test".into(),
                vec!["alice".into(), "bob".into()]
            ))
        );
        for end in 0..body.len() {
            assert!(decode(&body[..end]).is_none());
        }
        assert!(
            decode(
                &Message::new()
                    .write_string("tag")
                    .write_int32(u32::MAX)
                    .get_data()
            )
            .is_none()
        );
        assert_eq!(
            decode(&Message::new().write_string("tag").write_int32(0).get_data()),
            Some(("tag".into(), vec![]))
        );
    }
}
