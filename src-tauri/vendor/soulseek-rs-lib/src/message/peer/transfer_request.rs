use crate::{
    message::{Message, MessageHandler},
    peer::PeerSignal,
    types::Transfer,
};
use tokio::sync::mpsc::UnboundedSender;

pub struct TransferRequest;
impl MessageHandler<PeerSignal> for TransferRequest {
    fn get_code(&self) -> u32 {
        40
    }
    fn handle(&self, message: &mut Message, sender: UnboundedSender<PeerSignal>) {
        let transfer = Transfer::new_from_message(message);

        sender.send(PeerSignal::TransferRequest(transfer)).unwrap();
    }
}
