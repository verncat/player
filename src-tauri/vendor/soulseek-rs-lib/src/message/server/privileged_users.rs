use crate::debug;
use tokio::sync::mpsc::UnboundedSender;

use crate::{
    actor::server_actor::ServerSignal,
    message::{Message, MessageHandler},
};

pub struct PrivilegedUsersHandler;

impl MessageHandler<ServerSignal> for PrivilegedUsersHandler {
    fn get_code(&self) -> u32 {
        69
    }

    fn handle(&self, message: &mut Message, _sender: UnboundedSender<ServerSignal>) {
        let _number = message.read_int32();
        debug!("Number of privileged users: {}", _number);
    }
}
