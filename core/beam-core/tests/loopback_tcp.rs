//! Loopback tests: two in-process peers talking over real TCP sockets on localhost.

// Helpers here are test code too; a failed unwrap is how they report a broken setup.
#![allow(clippy::unwrap_used)]

use std::net::{Ipv4Addr, SocketAddr};

use beam_core::handshake::Role;
use beam_core::tcp::{self, TcpAcceptor};
use beam_core::{Cipher, Connection, ErrorCategory, Frame, FrameType, Handshake, StaticKeypair};
use tokio::net::TcpStream;

const TOKEN: &[u8; 16] = b"one-time-token!!";
const MIB: usize = 1024 * 1024;

fn key() -> StaticKeypair {
    StaticKeypair::generate().unwrap()
}

async fn acceptor() -> (TcpAcceptor, SocketAddr) {
    let acceptor = TcpAcceptor::bind((Ipv4Addr::LOCALHOST, 0).into())
        .await
        .unwrap();
    let addr = acceptor.local_addr().unwrap();
    (acceptor, addr)
}

/// Pairs over TCP; returns (scanner side, phone side).
async fn paired(
    scanner: &StaticKeypair,
    phone: &StaticKeypair,
) -> (Connection<TcpStream>, Connection<TcpStream>) {
    let (acceptor, addr) = acceptor().await;
    let i = Handshake::pairing_scanner(
        scanner,
        Cipher::AesGcm,
        Role::Initiator,
        TOKEN,
        phone.public(),
    )
    .unwrap();
    let r = Handshake::pairing_displayer(phone, Cipher::AesGcm, Role::Responder, TOKEN).unwrap();
    let (a, b) = tokio::join!(tcp::connect(addr, i), acceptor.accept(r));
    (a.unwrap(), b.unwrap().0)
}

fn numbered(n: u32) -> Frame {
    Frame::new(FrameType::Chunk, 1, n.to_be_bytes().to_vec()).unwrap()
}

fn number(frame: &Frame) -> u32 {
    u32::from_be_bytes(frame.payload[..4].try_into().unwrap())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn bulk_and_many_small_frames_arrive_intact() {
    let (scanner, phone) = (key(), key());
    let (mut sender, mut receiver) = paired(&scanner, &phone).await;
    let data: Vec<u8> = (0..=250u8).cycle().take(32 * MIB).collect();
    let expected = blake3::hash(&data);

    let send = tokio::spawn(async move {
        for piece in data.chunks(MIB) {
            sender
                .send(&Frame::new(FrameType::Chunk, 1, piece.to_vec()).unwrap())
                .await
                .unwrap();
        }
        for n in 0..2_000 {
            sender.send(&numbered(n)).await.unwrap();
        }
        sender.close().await.unwrap();
    });

    let mut hasher = blake3::Hasher::new();
    let mut small = 0u32;
    while let Some(frame) = receiver.recv().await.unwrap() {
        if frame.payload.len() == MIB {
            hasher.update(&frame.payload);
        } else {
            assert_eq!(number(&frame), small);
            small += 1;
        }
    }
    send.await.unwrap();
    assert_eq!(hasher.finalize(), expected);
    assert_eq!(small, 2_000);
}

#[tokio::test]
async fn trusted_devices_reconnect_with_the_known_device_handshake() {
    let (laptop, phone) = (key(), key());
    let (acceptor, addr) = acceptor().await;
    let i = Handshake::known_initiator(&laptop, Cipher::ChaChaPoly, phone.public()).unwrap();
    let r = Handshake::known_responder(&phone, Cipher::ChaChaPoly).unwrap();
    let (a, b) = tokio::join!(tcp::connect(addr, i), acceptor.accept(r));
    let (mut a, (mut b, _)) = (a.unwrap(), b.unwrap());
    assert_eq!(b.peer(), laptop.public());
    a.send(&numbered(7)).await.unwrap();
    assert_eq!(number(&b.recv().await.unwrap().unwrap()), 7);
}

#[tokio::test]
async fn an_impostor_listening_on_the_address_is_refused() {
    let (laptop, phone, impostor) = (key(), key(), key());
    let (acceptor, addr) = acceptor().await;
    let i = Handshake::known_initiator(&laptop, Cipher::AesGcm, phone.public()).unwrap();
    let r = Handshake::known_responder(&impostor, Cipher::AesGcm).unwrap();
    let (a, b) = tokio::join!(tcp::connect(addr, i), acceptor.accept(r));
    let err = a.err().or(b.err()).unwrap();
    assert!(matches!(
        err.category(),
        ErrorCategory::Crypto | ErrorCategory::Network
    ));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_dropped_connection_resumes_on_reconnect_without_gaps() {
    const TOTAL: u32 = 500;
    const DROP_AFTER: u32 = 137;
    let (laptop, phone) = (key(), key());
    let (mut sender, mut receiver) = paired(&laptop, &phone).await;

    // The sender's link dies partway through.
    for n in 0..DROP_AFTER {
        sender.send(&numbered(n)).await.unwrap();
    }
    drop(sender);
    let mut got = Vec::new();
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

    // Reconnect as trusted devices; the receiver says where to continue.
    let (acceptor, addr) = acceptor().await;
    let i = Handshake::known_initiator(&phone, Cipher::AesGcm, laptop.public()).unwrap();
    let r = Handshake::known_responder(&laptop, Cipher::AesGcm).unwrap();
    let (a, b) = tokio::join!(tcp::connect(addr, i), acceptor.accept(r));
    let (mut receiver, (mut sender, _)) = (a.unwrap(), b.unwrap());
    let resume_at = u32::try_from(got.len()).unwrap();
    receiver
        .send(&Frame::new(FrameType::Resume, 1, resume_at.to_be_bytes().to_vec()).unwrap())
        .await
        .unwrap();
    let request = sender.recv().await.unwrap().unwrap();
    assert_eq!(request.frame_type, FrameType::Resume);
    for n in number(&request)..TOTAL {
        sender.send(&numbered(n)).await.unwrap();
    }
    sender.close().await.unwrap();
    while let Some(frame) = receiver.recv().await.unwrap() {
        got.push(number(&frame));
    }
    assert_eq!(got, (0..TOTAL).collect::<Vec<_>>());
}
