use crate::{
    message::{Message, MessageHandler},
    path::SoulseekPath,
    peer::PeerSignal,
};
use tokio::sync::mpsc::UnboundedSender;

pub struct PlaceInQueueResponse;

impl MessageHandler<PeerSignal> for PlaceInQueueResponse {
    fn get_code(&self) -> u32 {
        43
    }

    fn handle(&self, message: &mut Message, sender: UnboundedSender<PeerSignal>) {
        let filename = SoulseekPath::from_wire(message.read_string());
        let place = message.read_int32();

        sender
            .send(PeerSignal::PlaceInQueueResponse { filename, place })
            .unwrap();
    }
}
