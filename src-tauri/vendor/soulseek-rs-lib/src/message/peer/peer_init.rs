use tokio::sync::mpsc::UnboundedSender;

use crate::{
    message::{Message, MessageHandler},
    peer::PeerSignal,
    trace,
};

pub struct PeerInit;
impl MessageHandler<PeerSignal> for PeerInit {
    fn get_code(&self) -> u32 {
        1
    }

    fn handle(&self, message: &mut Message, sender: UnboundedSender<PeerSignal>) {
        message.set_pointer(4);
        let _message_code = message.read_int8();
        let username = message.read_string();
        let _connection_type = message.read_string();
        let _token = message.read_int32();
        trace!(
            "PeerInit: username: {}, connection_type: {}, token: {}",
            username, _connection_type, _token
        );

        sender.send(PeerSignal::SetUsername(username)).unwrap();
    }
}
