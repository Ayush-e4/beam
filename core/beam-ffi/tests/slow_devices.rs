//! Devices that connect and never speak must not hold up the listener for anyone else.

// Test code reports a broken setup by panicking.
#![allow(clippy::unwrap_used, clippy::panic)]

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use beam_ffi::{
    DeclineReason, DeviceIdentity, Engine, Listener, ProgressListener, SendResult, SessionCipher,
    TrustedKeys,
};
use tempfile::TempDir;
use tokio::io::AsyncReadExt;
use tokio::net::TcpStream;
use tokio::time::timeout;

const CIPHER: SessionCipher = SessionCipher::AesGcm;
const PER_ADDRESS: usize = 4;

struct Quiet;

impl ProgressListener for Quiet {
    fn on_progress(&self, _: u64, _: u64) {}
}

struct Nobody;

impl TrustedKeys for Nobody {
    fn is_trusted(&self, _: Vec<u8>) -> bool {
        false
    }
}

struct Device {
    identity: Arc<DeviceIdentity>,
    engine: Arc<Engine>,
}

fn device() -> Device {
    let identity = DeviceIdentity::generate().unwrap();
    Device {
        engine: Engine::new(identity.clone()).unwrap(),
        identity,
    }
}

struct Phone {
    device: Device,
    listener: Arc<Listener>,
    _dir: TempDir,
    note: PathBuf,
}

async fn phone() -> Phone {
    let device = device();
    let listener = device
        .engine
        .listen(0, CIPHER, Arc::new(Nobody))
        .await
        .unwrap();
    let dir = TempDir::new().unwrap();
    let note = dir.path().join("note.txt");
    std::fs::write(&note, b"hello").unwrap();
    Phone {
        device,
        listener,
        _dir: dir,
        note,
    }
}

impl Phone {
    /// Connections that never say anything.
    async fn silent(&self, count: usize) -> Vec<TcpStream> {
        let mut streams = Vec::new();
        for _ in 0..count {
            let port = self.listener.port();
            streams.push(TcpStream::connect(("127.0.0.1", port)).await.unwrap());
        }
        // Let the listener accept them before anything else arrives.
        tokio::time::sleep(Duration::from_millis(200)).await;
        streams
    }

    /// A device offers a file and the person declines; returns how long it took.
    async fn offer_and_decline(&self) -> Duration {
        let sender = device();
        let path = self.note.to_string_lossy().into_owned();
        let outgoing = sender.engine.prepare_send(vec![path]).await.unwrap();
        let key = self.device.identity.public_key();
        let started = Instant::now();
        let send = outgoing.send_to(
            "127.0.0.1".into(),
            self.listener.port(),
            key,
            CIPHER,
            Arc::new(Quiet),
        );
        let decide = async {
            let offer = self.listener.next_offer().await.unwrap();
            offer.decline(DeclineReason::Busy).await.unwrap();
        };
        let (outcome, ()) = tokio::join!(send, decide);
        assert_eq!(
            outcome.unwrap(),
            SendResult::Declined {
                reason: DeclineReason::Busy
            }
        );
        started.elapsed()
    }
}

/// Whether the other end closed `stream` within two seconds.
async fn closed(stream: &mut TcpStream) -> bool {
    let mut byte = [0u8; 1];
    matches!(
        timeout(Duration::from_secs(2), stream.read(&mut byte)).await,
        Ok(Ok(0) | Err(_))
    )
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn silent_devices_do_not_hold_up_others() {
    let phone = phone().await;
    let _silent = phone.silent(PER_ADDRESS - 1).await;
    let took = Box::pin(phone.offer_and_decline()).await;
    assert!(took < Duration::from_secs(3), "took {took:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn one_address_gets_a_few_connections_at_a_time() {
    let phone = phone().await;
    let silent = phone.silent(PER_ADDRESS).await;
    let mut one_more = phone.silent(1).await.pop().unwrap();
    assert!(closed(&mut one_more).await, "over the cap, closed at once");

    drop(silent);
    tokio::time::sleep(Duration::from_millis(200)).await;
    let took = Box::pin(phone.offer_and_decline()).await;
    assert!(took < Duration::from_secs(3), "slots come back: {took:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn closing_the_listener_drops_connections_in_progress() {
    let phone = phone().await;
    let mut silent = phone.silent(1).await.pop().unwrap();
    let Phone { listener, .. } = phone;
    drop(listener);
    assert!(closed(&mut silent).await);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn waiting_can_be_cancelled_without_stopping_the_listener() {
    let phone = phone().await;
    let waited = timeout(Duration::from_millis(100), phone.listener.next_offer()).await;
    assert!(waited.is_err(), "nothing offered yet");
    let took = Box::pin(phone.offer_and_decline()).await;
    assert!(took < Duration::from_secs(3), "still listening: {took:?}");
}
