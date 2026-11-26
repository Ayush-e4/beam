//! A failed send says how it failed: refused (nothing listens there) or no answer at all.

// Test code reports a broken setup by panicking.
#![allow(clippy::unwrap_used, clippy::panic)]

use std::fs;
use std::net::TcpListener;
use std::sync::Arc;

use beam_ffi::{
    BeamError, DeviceIdentity, Engine, NetworkFailure, OutgoingTransfer, ProgressListener,
    SessionCipher,
};
use tempfile::TempDir;

struct Quiet;

impl ProgressListener for Quiet {
    fn on_progress(&self, _: u64, _: u64) {}
}

async fn outgoing(dir: &TempDir) -> (Arc<Engine>, Arc<OutgoingTransfer>) {
    let file = dir.path().join("note.txt");
    fs::write(&file, b"hello").unwrap();
    let engine = Engine::new(DeviceIdentity::generate().unwrap()).unwrap();
    let outgoing = engine
        .prepare_send(vec![file.to_string_lossy().into_owned()])
        .await
        .unwrap();
    (engine, outgoing)
}

async fn failure(outgoing: &OutgoingTransfer, host: &str, port: u16) -> NetworkFailure {
    let peer = DeviceIdentity::generate().unwrap().public_key();
    let result = outgoing
        .send_to(
            host.into(),
            port,
            peer,
            SessionCipher::ChaChaPoly,
            Arc::new(Quiet),
            None,
        )
        .await;
    match result {
        Err(BeamError::Network { failure, .. }) => failure,
        other => panic!("expected a network error, got {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_port_nothing_listens_on_is_refused() {
    let dir = TempDir::new().unwrap();
    let (_engine, outgoing) = outgoing(&dir).await;
    let closed = TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port(); // the listener is dropped, so nothing listens there now
    assert_eq!(
        failure(&outgoing, "127.0.0.1", closed).await,
        NetworkFailure::Refused
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_address_nothing_answers_is_no_answer() {
    let dir = TempDir::new().unwrap();
    let (_engine, outgoing) = outgoing(&dir).await;
    // 192.0.2.0/24 is reserved for documentation: nothing there ever answers. The system
    // either drops the attempt (a timeout) or says the network is unreachable.
    assert_eq!(
        failure(&outgoing, "192.0.2.1", 9).await,
        NetworkFailure::NoAnswer
    );
}
