//! Loopback tests: two in-process peers talking over QUIC on localhost.

// Helpers here are test code too; a failed unwrap is how they report a broken setup.
#![allow(clippy::unwrap_used)]

use std::net::{Ipv4Addr, SocketAddr};

use beam_core::handshake::Role;
use beam_core::quic::{QuicEndpoint, QuicStream};
use beam_core::{Cipher, Connection, ErrorCategory, Frame, FrameType, Handshake, StaticKeypair};

const TOKEN: &[u8; 16] = b"one-time-token!!";
const MIB: usize = 1024 * 1024;

fn key() -> StaticKeypair {
    StaticKeypair::generate().unwrap()
}

fn endpoint() -> (QuicEndpoint, SocketAddr) {
    let endpoint = QuicEndpoint::bind((Ipv4Addr::LOCALHOST, 0).into()).unwrap();
    let addr = endpoint.local_addr().unwrap();
    (endpoint, addr)
}

fn numbered(n: u32) -> Frame {
    Frame::new(FrameType::Chunk, 1, n.to_be_bytes().to_vec()).unwrap()
}

fn number(frame: &Frame) -> u32 {
    u32::from_be_bytes(frame.payload[..4].try_into().unwrap())
}

/// Reads frames until the peer closes, then closes our side too.
async fn drain(conn: &mut Connection<QuicStream>) -> Vec<Frame> {
    let mut frames = Vec::new();
    while let Some(frame) = conn.recv().await.unwrap() {
        frames.push(frame);
    }
    conn.close().await.unwrap();
    frames
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pairing_then_bulk_and_many_small_frames_arrive_intact() {
    let (scanner, phone) = (key(), key());
    let (client, _) = endpoint();
    let (server, addr) = endpoint();
    let i = Handshake::pairing_scanner(
        &scanner,
        Cipher::AesGcm,
        Role::Initiator,
        TOKEN,
        phone.public(),
    )
    .unwrap();
    let r = Handshake::pairing_displayer(&phone, Cipher::AesGcm, Role::Responder, TOKEN).unwrap();
    let (a, b) = tokio::join!(client.connect(addr, i), server.accept(r));
    let (mut sender, (mut receiver, _)) = (a.unwrap(), b.unwrap());
    assert_eq!(sender.verification_code(), receiver.verification_code());

    let data: Vec<u8> = (0..=250u8).cycle().take(32 * MIB).collect();
    let expected = blake3::hash(&data);
    let send = tokio::spawn(async move {
        for piece in data.chunks(MIB) {
            let frame = Frame::new(FrameType::Chunk, 1, piece.to_vec()).unwrap();
            sender.send(&frame).await.unwrap();
        }
        for n in 0..2_000 {
            sender.send(&numbered(n)).await.unwrap();
        }
        sender.close().await.unwrap();
        // Wait for the receiver to finish before the connection is dropped.
        assert!(sender.recv().await.unwrap().is_none());
    });

    let frames = drain(&mut receiver).await;
    send.await.unwrap();
    let mut hasher = blake3::Hasher::new();
    let mut small = Vec::new();
    for frame in &frames {
        if frame.payload.len() == MIB {
            hasher.update(&frame.payload);
        } else {
            small.push(number(frame));
        }
    }
    assert_eq!(hasher.finalize(), expected);
    assert_eq!(small, (0..2_000).collect::<Vec<_>>());
}

#[tokio::test]
async fn one_endpoint_serves_several_trusted_devices() {
    let phone = key();
    let (server, addr) = endpoint();
    let laptops = [key(), key(), key()];
    for laptop in &laptops {
        let (client, _) = endpoint();
        let i = Handshake::known_initiator(laptop, Cipher::ChaChaPoly, phone.public()).unwrap();
        let r = Handshake::known_responder(&phone, Cipher::ChaChaPoly).unwrap();
        let (a, b) = tokio::join!(client.connect(addr, i), server.accept(r));
        let (mut a, (mut b, _)) = (a.unwrap(), b.unwrap());
        assert_eq!(b.peer(), laptop.public());
        a.send(&numbered(5)).await.unwrap();
        assert_eq!(number(&b.recv().await.unwrap().unwrap()), 5);
    }
}

#[tokio::test]
async fn an_impostor_is_refused() {
    let (laptop, phone, impostor) = (key(), key(), key());
    let (client, _) = endpoint();
    let (server, addr) = endpoint();
    let i = Handshake::known_initiator(&laptop, Cipher::AesGcm, phone.public()).unwrap();
    let r = Handshake::known_responder(&impostor, Cipher::AesGcm).unwrap();
    let (a, b) = tokio::join!(client.connect(addr, i), server.accept(r));
    let err = a.err().or(b.err()).unwrap();
    assert!(matches!(
        err.category(),
        ErrorCategory::Crypto | ErrorCategory::Network
    ));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_lost_link_resumes_on_reconnect_without_gaps() {
    const TOTAL: u32 = 500;
    let (laptop, phone) = (key(), key());
    let (server, addr) = endpoint();
    let mut got = Vec::new();

    // First link: the sender's endpoint vanishes partway through.
    {
        let (client, _) = endpoint();
        let i = Handshake::known_initiator(&laptop, Cipher::AesGcm, phone.public()).unwrap();
        let r = Handshake::known_responder(&phone, Cipher::AesGcm).unwrap();
        let (a, b) = tokio::join!(client.connect(addr, i), server.accept(r));
        let (mut sender, (mut receiver, _)) = (a.unwrap(), b.unwrap());
        for n in 0..120 {
            sender.send(&numbered(n)).await.unwrap();
        }
        while got.len() < 60 {
            got.push(number(&receiver.recv().await.unwrap().unwrap()));
        }
        client.close();
        loop {
            match receiver.recv().await {
                Ok(Some(frame)) => got.push(number(&frame)),
                Ok(None) => break,
                Err(err) => {
                    assert_eq!(err.category(), ErrorCategory::Network);
                    break;
                }
            }
        }
    }

    // Second link: the receiver says where to continue.
    let (client, _) = endpoint();
    let i = Handshake::known_initiator(&laptop, Cipher::AesGcm, phone.public()).unwrap();
    let r = Handshake::known_responder(&phone, Cipher::AesGcm).unwrap();
    let (a, b) = tokio::join!(client.connect(addr, i), server.accept(r));
    let (mut sender, (mut receiver, _)) = (a.unwrap(), b.unwrap());
    let resume_at = u32::try_from(got.len()).unwrap();
    let resume = Frame::new(FrameType::Resume, 1, resume_at.to_be_bytes().to_vec()).unwrap();
    receiver.send(&resume).await.unwrap();
    let request = sender.recv().await.unwrap().unwrap();
    let send = tokio::spawn(async move {
        for n in number(&request)..TOTAL {
            sender.send(&numbered(n)).await.unwrap();
        }
        sender.close().await.unwrap();
        assert!(sender.recv().await.unwrap().is_none());
    });
    got.extend(drain(&mut receiver).await.iter().map(number));
    send.await.unwrap();
    assert_eq!(got, (0..TOTAL).collect::<Vec<_>>());
}
