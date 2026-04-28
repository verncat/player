use crate::debug;
use tokio::sync::mpsc::UnboundedSender;

use crate::{
    actor::server_actor::ServerSignal,
    message::{Message, MessageHandler},
};

pub struct ParentMinSpeedHandler;

impl MessageHandler<ServerSignal> for ParentMinSpeedHandler {
    fn get_code(&self) -> u32 {
        83
    }

    fn handle(&self, message: &mut Message, sender: UnboundedSender<ServerSignal>) {
        let _ = sender;
        let _number = message.read_int32();
        debug!("Parent min speed: {}", _number);
    }
}
