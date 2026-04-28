use crate::{debug, info};
use tokio::sync::mpsc::UnboundedSender;

use crate::{
    actor::server_actor::ServerSignal, message::Message, message::handlers::MessageHandler,
};

pub struct FileSearchHandler;

impl MessageHandler<ServerSignal> for FileSearchHandler {
    fn get_code(&self) -> u32 {
        26
    }
    fn handle(&self, message: &mut Message, _sender: UnboundedSender<ServerSignal>) {
        debug!("Handling file search message");
        let _username = message.read_string();
        let _token = message.read_int32();
        let _query = message.read_string();
        info!(
            "Message search username:{}, token: {}, query: {}",
            _username, _token, _query
        );
    }
}
