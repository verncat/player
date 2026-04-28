use crate::actor::server_actor::ServerSignal;
use crate::message::{Message, MessageHandler};
use crate::peer::Peer;
use tokio::sync::mpsc::UnboundedSender;
pub struct ConnectToPeerHandler;

impl MessageHandler<ServerSignal> for ConnectToPeerHandler {
    fn get_code(&self) -> u32 {
        18
    }
    fn handle(&self, message: &mut Message, sender: UnboundedSender<ServerSignal>) {
        let peer = Peer::new_from_message(message);
        sender.send(ServerSignal::ConnectToPeer(peer)).unwrap();
    }
}
