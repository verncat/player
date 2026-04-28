use crate::info;
use crate::{
    message::{Message, MessageHandler},
    peer::PeerSignal,
    types::UploadFailed,
};
use tokio::sync::mpsc::UnboundedSender;

pub struct UploadFailedHandler;
impl MessageHandler<PeerSignal> for UploadFailedHandler {
    fn get_code(&self) -> u32 {
        46
    }
    fn handle(&self, message: &mut Message, _sender: UnboundedSender<PeerSignal>) {
        let _upload_failed = UploadFailed::new_from_message(message);
        info!("Upload failed for ${}", _upload_failed.filename);
    }
}
