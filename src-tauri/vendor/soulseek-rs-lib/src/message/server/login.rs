use crate::{actor::server_actor::ServerSignal, debug, info, message::Message};
use tokio::sync::mpsc::UnboundedSender;

use crate::message::MessageHandler;

pub struct LoginHandler;

impl MessageHandler<ServerSignal> for LoginHandler {
    fn get_code(&self) -> u32 {
        1
    }

    fn handle(&self, message: &mut Message, sender: UnboundedSender<ServerSignal>) {
        let response = message.read_int8();

        if response != 1 {
            return sender.send(ServerSignal::LoginStatus(false)).unwrap();
        }

        info!("Login successful");
        let _greeting = message.read_string();
        debug!("Server greeting: {:?}", _greeting);

        let _own_ip = message.read_int32();
        debug!("Own IP: {}", _own_ip);

        let _password_hash = message.read_string();
        debug!("Password hash: {:?}", _password_hash);

        let _supporter = message.read_bool();
        debug!("Supporter status: {}", _supporter);

        sender.send(ServerSignal::LoginStatus(true)).unwrap();
    }
}

// fn build_login_response_message() -> Message {
//     return Message::new_with_data([
//         50, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 0, 81, 170, 162, 77, 32, 0, 0, 0, 101, 102, 99, 97,
//         51, 52, 102, 99, 52, 99, 56, 98, 101, 56, 98, 55, 101, 102, 51, 56, 97, 102, 50, 54, 50,
//         52, 100, 101, 53, 52, 54, 52, 0,
//     ]);
// }
// #[test]
// fn test_can_handle() {
//     assert_eq!(true, LoginHandler.can_handle(build_login_response_message());
//     );
// }
