use super::*;
use crate::message::Message;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[tokio::test]
async fn interest_discovery_uses_one_login_and_correlates_results() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut received = Vec::new();
        loop {
            let Ok(size) = socket.read_u32_le().await else {
                break;
            };
            assert!(size < 4096);
            let mut bytes = vec![0; size as usize];
            socket.read_exact(&mut bytes).await.unwrap();
            let mut packet = Message::new_with_data(bytes);
            let code = packet.read_int32();
            if code == 1 {
                socket
                    .write_all(
                        &Message::new()
                            .write_int32(1)
                            .write_int8(1)
                            .write_string("welcome")
                            .write_int32(0)
                            .write_string("")
                            .write_bool(false)
                            .get_buffer(),
                    )
                    .await
                    .unwrap();
                received.push((code, "login".to_owned()));
            } else if matches!(code, 51 | 52 | 112) {
                let item = packet.read_string();
                received.push((code, item.clone()));
                if code == 112 {
                    // A response for another item must never satisfy this request.
                    socket
                        .write_all(
                            &Message::new()
                                .write_int32(112)
                                .write_string("unrelated")
                                .write_int32(1)
                                .write_string("wrong-user")
                                .get_buffer(),
                        )
                        .await
                        .unwrap();
                    let mut response = Message::new();
                    response.write_int32(112).write_string(&item);
                    if item.ends_with("empty") {
                        response.write_int32(0);
                    } else {
                        response
                            .write_int32(3)
                            .write_string("bob")
                            .write_string("alice")
                            .write_string("bob");
                    }
                    socket.write_all(&response.get_buffer()).await.unwrap();
                }
            }
        }
        received
    });
    let mut settings = ClientSettings::new("synthetic-user", "synthetic-password");
    settings.server_address = crate::PeerAddress::new("127.0.0.1".into(), port);
    settings.enable_listen = false;
    settings.reconnect_settings = ReconnectSettings::Disabled;
    settings.search_rate_limit_settings = None;
    let client = Client::with_settings(settings);
    let (a, b) = tokio::join!(client.connect(), client.connect());
    a.unwrap();
    b.unwrap();
    client
        .set_discovery_interest(
            Some("adhdplayer-v1-old".into()),
            Some("adhdplayer-v1-new".into()),
        )
        .await
        .unwrap();
    assert_eq!(
        client
            .find_interest_users("adhdplayer-v1-new")
            .await
            .unwrap(),
        vec!["alice", "bob"]
    );
    assert!(
        client
            .find_interest_users("adhdplayer-v1-empty")
            .await
            .unwrap()
            .is_empty()
    );
    client
        .set_discovery_interest(Some("adhdplayer-v1-new".into()), None)
        .await
        .unwrap();
    client.disconnect();
    let packets = tokio::time::timeout(Duration::from_secs(3), server)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        packets,
        vec![
            (1, "login".into()),
            (52, "adhdplayer-v1-old".into()),
            (51, "adhdplayer-v1-new".into()),
            (112, "adhdplayer-v1-new".into()),
            (112, "adhdplayer-v1-empty".into()),
            (52, "adhdplayer-v1-new".into()),
        ]
    );
}
