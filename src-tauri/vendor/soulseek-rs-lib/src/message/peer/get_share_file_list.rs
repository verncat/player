use crate::{
    message::{Message, MessageHandler, server::MessageFactory},
    peer::PeerSignal,
};
use tokio::sync::mpsc::UnboundedSender;

pub struct GetShareFileList {
    pub shared_folders: u32,
    pub shared_files: u32,
}
impl MessageHandler<PeerSignal> for GetShareFileList {
    fn get_code(&self) -> u32 {
        4
    }
    fn handle(&self, _message: &mut Message, sender: UnboundedSender<PeerSignal>) {
        let message = MessageFactory::build_shared_folders_message(self.shared_folders, self.shared_files);

        sender.send(PeerSignal::SendMessage(message)).unwrap();
    }
}
