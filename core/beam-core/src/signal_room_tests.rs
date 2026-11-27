use futures_util::{SinkExt, StreamExt};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio_tungstenite::tungstenite::protocol::CloseFrame;

use super::*;
use crate::ErrorCategory;

const ROOM: [u8; 16] = [0xAB; 16];

/// A one-device stand-in for the service: checks the path, says `joined`, then echoes each
/// binary message back as if from the other device, until it gets "close <code>".
async fn fake_service() -> String {
    fake_service_greeting(Message::text(r#"{"type":"joined","peers":0}"#)).await
}

/// [`fake_service`] that greets each device with `greeting` instead.
async fn fake_service_greeting(greeting: Message) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        while let Ok((tcp, _)) = listener.accept().await {
            let greeting = greeting.clone();
            tokio::spawn(async move {
                #[allow(clippy::result_large_err)] // the handshake callback's signature
                let check = |request: &http::Request<()>, response| {
                    assert_eq!(request.uri().path(), format!("/rooms/{}", "ab".repeat(16)));
                    Ok(response)
                };
                let mut socket = tokio_tungstenite::accept_hdr_async(tcp, check)
                    .await
                    .unwrap();
                socket.send(greeting).await.unwrap();
                while let Some(Ok(message)) = socket.next().await {
                    let reply = match &message {
                        Message::Binary(bytes) if bytes.starts_with(b"close ") => {
                            let code: u16 =
                                std::str::from_utf8(&bytes[6..]).unwrap().parse().unwrap();
                            Message::Close(Some(CloseFrame {
                                code: code.into(),
                                reason: "bye".into(),
                            }))
                        }
                        Message::Binary(_) => message.clone(),
                        _ => continue,
                    };
                    let _ = socket.send(reply).await;
                }
            });
        }
    });
    format!("ws://{addr}")
}

#[tokio::test]
async fn a_device_joins_posts_and_hears_back() {
    let service = fake_service().await;
    let mut room = SignalRoom::join(&format!("{service}/"), &ROOM)
        .await
        .unwrap();
    assert_eq!(
        room.next().await.unwrap(),
        Some(RoomEvent::Joined { peers: 0 })
    );
    room.post(vec![1, 2, 3]).await.unwrap();
    assert_eq!(
        room.next().await.unwrap(),
        Some(RoomEvent::Message(vec![1, 2, 3]))
    );
    room.leave().await.unwrap();
}

#[tokio::test]
async fn the_rooms_close_codes_say_why() {
    let service = fake_service().await;
    for (code, expected) in [(4001, Some("full")), (4002, Some("expired")), (1000, None)] {
        let mut room = SignalRoom::join(&service, &ROOM).await.unwrap();
        room.next().await.unwrap();
        room.post(format!("close {code}").into_bytes())
            .await
            .unwrap();
        match (room.next().await, expected) {
            (Err(BeamError::Signaling(why)), Some(word)) => assert!(why.contains(word), "{why}"),
            (Ok(None), None) => {}
            (other, _) => panic!("code {code}: {other:?}"),
        }
    }
}

#[tokio::test]
async fn bad_addresses_refusals_and_big_messages_are_errors() {
    for service in ["https://signal.example", "signal.example", "ws://"] {
        let err = SignalRoom::join(service, &ROOM).await.err().unwrap();
        assert!(matches!(err, BeamError::Malformed(_)), "{service}: {err}");
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let (mut tcp, _) = listener.accept().await.unwrap();
        let mut request = [0u8; 1024];
        let _ = tcp.read(&mut request).await;
        let response = "HTTP/1.1 400 Bad Request\r\ncontent-length: 8\r\n\r\nbad_room";
        tcp.write_all(response.as_bytes()).await.unwrap();
    });
    let err = SignalRoom::join(&format!("ws://{addr}"), &ROOM)
        .await
        .err()
        .unwrap();
    assert!(
        matches!(&err, BeamError::Signaling(why) if why.contains("400")),
        "{err}"
    );
    assert_eq!(err.category(), ErrorCategory::Network);

    let unused = TcpListener::bind("127.0.0.1:0")
        .await
        .unwrap()
        .local_addr()
        .unwrap();
    let err = SignalRoom::join(&format!("ws://{unused}"), &ROOM)
        .await
        .err()
        .unwrap();
    assert_eq!(err.category(), ErrorCategory::Network);

    let mut room = SignalRoom::join(&fake_service().await, &ROOM)
        .await
        .unwrap();
    assert!(room.post(vec![0; MAX_MESSAGE_LEN + 1]).await.is_err());
}

#[tokio::test]
async fn delivering_joins_posts_and_goes_unless_turned_away() {
    SignalRoom::deliver(&fake_service().await, &ROOM, vec![7; 64])
        .await
        .unwrap();

    let full = Message::Close(Some(CloseFrame {
        code: 4001.into(),
        reason: "full".into(),
    }));
    let err = SignalRoom::deliver(&fake_service_greeting(full).await, &ROOM, vec![7])
        .await
        .err()
        .unwrap();
    assert!(
        matches!(&err, BeamError::Signaling(why) if why.contains("full")),
        "{err}"
    );

    let silent = Message::text(r#"{"type":"news"}"#);
    let service = fake_service_greeting(silent).await;
    let err = tokio::time::timeout(
        std::time::Duration::from_millis(300),
        SignalRoom::deliver(&service, &ROOM, vec![7]),
    )
    .await;
    assert!(err.is_err(), "waits for the room to say it's in");
}

#[test]
fn notices_are_read_and_unknown_ones_skipped() {
    assert_eq!(
        notice(r#"{"type":"joined","peers":1}"#),
        Some(RoomEvent::Joined { peers: 1 })
    );
    assert_eq!(
        notice(r#"{ "peers" : 0 , "type" : "joined" }"#),
        Some(RoomEvent::Joined { peers: 0 })
    );
    assert_eq!(
        notice(r#"{"type":"peer-joined"}"#),
        Some(RoomEvent::PeerJoined)
    );
    assert_eq!(notice(r#"{"type":"peer-left"}"#), Some(RoomEvent::PeerLeft));
    assert_eq!(
        notice(r#"{"type":"buffer-full"}"#),
        Some(RoomEvent::BufferFull)
    );
    for skipped in [r#"{"type":"news"}"#, r#"{"type":"joined"}"#, "not json", ""] {
        assert_eq!(notice(skipped), None, "{skipped:?}");
    }
}
