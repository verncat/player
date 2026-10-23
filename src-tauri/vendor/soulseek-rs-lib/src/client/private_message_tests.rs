use super::*;
use crate::message::Message;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[tokio::test]
async fn private_messages_send_receive_and_acknowledge_on_the_existing_connection() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut acknowledgements = 0;
        loop {
            let size = socket.read_u32_le().await.unwrap();
            assert!(size < 8192);
            let mut data = vec![0; size as usize];
            socket.read_exact(&mut data).await.unwrap();
            let mut packet = Message::new_with_data(data);
            match packet.read_int32() {
                1 => {
                    socket.write_all(&Message::new().write_int32(1).write_int8(1)
                        .write_string("welcome").write_int32(0).write_string("").write_bool(false).get_buffer()).await.unwrap();
                }
                22 => {
                    assert_eq!(packet.read_string(), "alice");
                    assert_eq!(packet.read_string(), "Hello 🎵");
                    let reply = Message::new().write_int32(22).write_int32(77).write_int32(1700000000)
                        .write_string("alice").write_string("Hi 🎵").write_bool(true).get_buffer();
                    // Exercise framing across TCP reads and a server replay of the same PM.
                    socket.write_all(&reply[..5]).await.unwrap();
                    tokio::time::sleep(Duration::from_millis(30)).await;
                    socket.write_all(&reply[5..]).await.unwrap();
                    socket.write_all(&reply).await.unwrap();
                }
                23 => {
                    assert_eq!(packet.read_int32(), 77);
                    acknowledgements += 1;
                    if acknowledgements == 2 { return; }
                }
                _ => {}
            }
        }
    });
    let mut settings = ClientSettings::new("synthetic-user", "synthetic-password");
    settings.server_address = crate::PeerAddress::new("127.0.0.1".into(), port);
    settings.enable_listen = false;
    settings.reconnect_settings = ReconnectSettings::Disabled;
    let client = Client::with_settings(settings);
    let received = Arc::new(Mutex::new(Vec::new()));
    let stored = received.clone();
    client.set_private_message_handler(Arc::new(move |message| {
        stored.lock().unwrap().push((message.id, message.username.clone(), message.message.clone()));
        true
    }));
    client.connect().await.unwrap();
    assert!(client.send_private_message("alice", " ").await.is_err());
    client.send_private_message("alice", "Hello 🎵").await.unwrap();
    tokio::time::timeout(Duration::from_secs(5), server).await.unwrap().unwrap();
    assert_eq!(*received.lock().unwrap(), vec![
        (77, "alice".into(), "Hi 🎵".into()), (77, "alice".into(), "Hi 🎵".into()),
    ]);
    client.disconnect();
    assert!(client.send_private_message("alice", "after disconnect").await.is_err());
}
