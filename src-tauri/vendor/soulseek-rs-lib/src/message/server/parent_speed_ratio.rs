use crate::debug;
use tokio::sync::mpsc::UnboundedSender;

use crate::{
    actor::server_actor::ServerSignal,
    message::{Message, MessageHandler},
};

pub struct ParentSpeedRatioHandler;

// The server sends us a speed ratio determining the number of children we can have in the distributed network. The maximum number of children is our upload speed divided by the speed ratio.
impl MessageHandler<ServerSignal> for ParentSpeedRatioHandler {
    fn get_code(&self) -> u32 {
        84
    }

    fn handle(&self, message: &mut Message, _sender: UnboundedSender<ServerSignal>) {
        let _number = message.read_int32();
        debug!("Parent speed ratio: {}", _number);
    }
}
