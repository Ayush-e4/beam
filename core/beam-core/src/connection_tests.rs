use tokio::io::{DuplexStream, duplex};

use super::*;
use crate::ErrorCategory;
use crate::frame::FrameType;
use crate::handshake::Cipher;
use crate::keys::StaticKeypair;

const TOKEN: &[u8; 16] = b"one-time-token!!";
const PIPE_CAPACITY: usize = 256 * 1024;

fn keys() -> (StaticKeypair, StaticKeypair) {
    (
        StaticKeypair::generate().unwrap(),
        StaticKeypair::generate().unwrap(),
    )
}

async fn connected() -> (Connection<DuplexStream>, Connection<DuplexStream>) {
    let (scanner, phone) = keys();
    let (a, b) = duplex(PIPE_CAPACITY);
    let i = Handshake::pairing_initiator(&scanner, Cipher::AesGcm, TOKEN, phone.public()).unwrap();
    let r = Handshake::pairing_responder(&phone, Cipher::AesGcm, TOKEN).unwrap();
    let (a, b) = tokio::join!(Connection::establish(a, i), Connection::establish(b, r));
    (a.unwrap(), b.unwrap())
}

fn ping(n: u8) -> Frame {
    Frame::new(FrameType::Ping, 0, vec![n]).unwrap()
}

#[tokio::test]
async fn frames_round_trip_and_codes_match() {
    let (mut a, mut b) = connected().await;
    assert_eq!(a.verification_code(), b.verification_code());
    a.send(&ping(1)).await.unwrap();
    a.send(&ping(2)).await.unwrap();
    assert_eq!(b.recv().await.unwrap(), Some(ping(1)));
    assert_eq!(b.recv().await.unwrap(), Some(ping(2)));
    b.send(&ping(3)).await.unwrap();
    assert_eq!(a.recv().await.unwrap(), Some(ping(3)));
}

#[tokio::test]
async fn clean_close_ends_the_stream() {
    let (mut a, mut b) = connected().await;
    a.send(&ping(1)).await.unwrap();
    a.close().await.unwrap();
    assert_eq!(b.recv().await.unwrap(), Some(ping(1)));
    assert_eq!(b.recv().await.unwrap(), None);
}

#[tokio::test]
async fn cut_off_mid_record_is_a_network_error() {
    // Drive one side by hand so we can send half a record and hang up.
    let (scanner, phone) = keys();
    let (mut raw, b) = duplex(PIPE_CAPACITY);
    let r = Handshake::pairing_responder(&phone, Cipher::AesGcm, TOKEN).unwrap();
    let receiver = tokio::spawn(async move {
        let mut b = Connection::establish(b, r).await?;
        b.recv().await
    });
    let mut i =
        Handshake::pairing_initiator(&scanner, Cipher::AesGcm, TOKEN, phone.public()).unwrap();
    write_handshake_message(&mut raw, &i.write_message().unwrap())
        .await
        .unwrap();
    i.read_message(&read_handshake_message(&mut raw).await.unwrap())
        .unwrap();
    write_handshake_message(&mut raw, &i.write_message().unwrap())
        .await
        .unwrap();
    let mut session = Session::new(i.finish().unwrap());
    let mut wire = Vec::new();
    session.seal(&ping(9), &mut wire).unwrap();
    raw.write_all(&wire[..wire.len() / 2]).await.unwrap();
    drop(raw);

    let err = receiver.await.unwrap().unwrap_err();
    assert_eq!(err.category(), ErrorCategory::Network);
}

#[tokio::test]
async fn peer_hanging_up_during_handshake_is_a_network_error() {
    let (_, phone) = keys();
    let (raw, b) = duplex(PIPE_CAPACITY);
    drop(raw);
    let r = Handshake::pairing_responder(&phone, Cipher::AesGcm, TOKEN).unwrap();
    let err = Connection::establish(b, r).await.err().unwrap();
    assert_eq!(err.category(), ErrorCategory::Network);
}

#[tokio::test]
async fn garbage_handshake_is_a_crypto_error() {
    let (_, phone) = keys();
    let (mut raw, b) = duplex(PIPE_CAPACITY);
    raw.write_all(&[0, 5, 1, 2, 3, 4, 5]).await.unwrap();
    let r = Handshake::pairing_responder(&phone, Cipher::AesGcm, TOKEN).unwrap();
    let err = Connection::establish(b, r).await.err().unwrap();
    assert_eq!(err.category(), ErrorCategory::Crypto);
}
