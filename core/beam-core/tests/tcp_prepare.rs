//! Preparing a socket before it connects (how the app sends a connection over one network).

// A failed unwrap is how test code reports a broken setup.
#![allow(clippy::unwrap_used)]

use std::io;
use std::net::Ipv4Addr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use beam_core::handshake::Role;
use beam_core::tcp::{self, PrepareSocket, TcpAcceptor};
use beam_core::{Cipher, ErrorCategory, Handshake, StaticKeypair};

const TOKEN: &[u8; 16] = b"one-time-token!!";

fn handshakes() -> (Handshake, Handshake) {
    let (scanner, phone) = (
        StaticKeypair::generate().unwrap(),
        StaticKeypair::generate().unwrap(),
    );
    let initiator = Handshake::pairing_scanner(
        &scanner,
        Cipher::ChaChaPoly,
        Role::Initiator,
        TOKEN,
        phone.public(),
    )
    .unwrap();
    let responder =
        Handshake::pairing_displayer(&phone, Cipher::ChaChaPoly, Role::Responder, TOKEN).unwrap();
    (initiator, responder)
}

#[tokio::test]
async fn the_socket_is_prepared_before_it_connects() {
    let acceptor = TcpAcceptor::bind((Ipv4Addr::LOCALHOST, 0).into())
        .await
        .unwrap();
    let addr = acceptor.local_addr().unwrap();
    let prepared = Arc::new(AtomicBool::new(false));
    let seen = prepared.clone();
    let prepare: PrepareSocket = Arc::new(move |socket| {
        socket.set_nodelay(true)?; // a socket, not yet a stream
        seen.store(true, Ordering::SeqCst);
        Ok(())
    });
    let (initiator, responder) = handshakes();
    let (connected, accepted) = tokio::join!(
        tcp::connect_prepared(addr, initiator, Some(prepare)),
        acceptor.accept(responder)
    );
    connected.unwrap();
    accepted.unwrap();
    assert!(prepared.load(Ordering::SeqCst));
}

#[tokio::test]
async fn a_socket_that_cant_be_prepared_is_a_network_error() {
    let acceptor = TcpAcceptor::bind((Ipv4Addr::LOCALHOST, 0).into())
        .await
        .unwrap();
    let prepare: PrepareSocket = Arc::new(|_| {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "no such network",
        ))
    });
    let (initiator, _) = handshakes();
    let result =
        tcp::connect_prepared(acceptor.local_addr().unwrap(), initiator, Some(prepare)).await;
    let error = result.err().unwrap();
    assert_eq!(error.category(), ErrorCategory::Network);
    assert!(error.to_string().contains("no such network"), "{error}");
}
