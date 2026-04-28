use crate::debug;
use crate::{
    actor::server_actor::ServerSignal,
    message::{Message, MessageHandler},
};
use tokio::sync::mpsc::UnboundedSender;

pub struct ExcludedSearchPhrasesHandler;

impl MessageHandler<ServerSignal> for ExcludedSearchPhrasesHandler {
    fn get_code(&self) -> u32 {
        160
    }

    fn handle(&self, message: &mut Message, _sender: UnboundedSender<ServerSignal>) {
        let item_count = message.read_int32();

        let mut exluded_phrases: Vec<String> = Vec::new();
        for _ in 0..item_count {
            // Read the file name, size, and path (structure can vary)
            let phrase = message.read_string();
            exluded_phrases.push(phrase);
        }
        debug!("Excluded search phrases: {:?}", exluded_phrases);
    }
}
