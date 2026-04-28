use crate::debug;
use tokio::sync::mpsc::UnboundedSender;

use crate::{
    actor::server_actor::ServerSignal,
    message::{Message, MessageHandler},
};

pub struct WishListIntervalHandler;

// The server tells us the wishlist search interval.
// This interval is almost always 12 minutes, or 2 minutes for privileged users.
impl MessageHandler<ServerSignal> for WishListIntervalHandler {
    fn get_code(&self) -> u32 {
        104
    }

    fn handle(&self, message: &mut Message, _sender: UnboundedSender<ServerSignal>) {
        let _number = message.read_int32();
        debug!("Wishlist search interval: {} in seconds", _number);
    }
}
