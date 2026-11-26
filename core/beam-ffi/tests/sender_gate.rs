//! Offers from devices that aren't trusted: flagged, rate-limited, and blocked after repeated
//! declines; trusted devices are never limited.

// Test code reports a broken setup by panicking.
#![allow(clippy::unwrap_used, clippy::panic)]

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use beam_ffi::{
    BeamError, DeclineReason, DeviceIdentity, Engine, Listener, ProgressListener, SendResult,
    SessionCipher, TrustedKeys,
};
use tempfile::TempDir;
use tokio::time::timeout;

const CIPHER: SessionCipher = SessionCipher::AesGcm;

struct Quiet;

impl ProgressListener for Quiet {
    fn on_progress(&self, _: u64, _: u64) {}
}

struct Trusts(Vec<Vec<u8>>);

impl TrustedKeys for Trusts {
    fn is_trusted(&self, public_key: Vec<u8>) -> bool {
        self.0.contains(&public_key)
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

/// A phone listening on loopback that trusts `friends`, and a file to offer it.
struct Phone {
    device: Device,
    listener: Arc<Listener>,
    _dir: TempDir,
    note: PathBuf,
}

async fn phone(friends: &[&Device]) -> Phone {
    let device = device();
    let keys = friends.iter().map(|d| d.identity.public_key()).collect();
    let listener = device
        .engine
        .listen(0, CIPHER, Arc::new(Trusts(keys)))
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
    async fn offer_from(&self, sender: &Device) -> Result<SendResult, BeamError> {
        let path = self.note.to_string_lossy().into_owned();
        let outgoing = sender.engine.prepare_send(vec![path]).await.unwrap();
        let key = self.device.identity.public_key();
        let port = self.listener.port();
        outgoing
            .send_to("127.0.0.1".into(), port, key, CIPHER, Arc::new(Quiet), None)
            .await
    }

    /// `sender` offers and the person declines with `reason`; returns whether the offer was
    /// flagged as coming from a trusted device.
    async fn offer_and_decline(&self, sender: &Device, reason: DeclineReason) -> bool {
        let decide = async {
            let offer = self.listener.next_offer().await.unwrap();
            offer.decline(reason).await.unwrap();
            offer.from_trusted_device()
        };
        let (sent, trusted) = tokio::join!(self.offer_from(sender), decide);
        assert_eq!(sent.unwrap(), SendResult::Declined { reason });
        trusted
    }

    /// Whether an offer from `sender` reaches the app within a second.
    async fn reaches_the_app(&self, sender: &Device) -> (Result<SendResult, BeamError>, bool) {
        let wait = timeout(Duration::from_secs(1), self.listener.next_offer());
        let (sent, offered) = tokio::join!(self.offer_from(sender), wait);
        (sent, offered.is_ok())
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_address_is_blocked_after_three_declines() {
    let stranger = device();
    let phone = phone(&[]).await;
    for _ in 0..3 {
        let trusted =
            Box::pin(phone.offer_and_decline(&stranger, DeclineReason::UserDeclined)).await;
        assert!(!trusted, "a stranger is flagged as unknown");
    }
    let (sent, reached) = Box::pin(phone.reaches_the_app(&stranger)).await;
    assert!(!reached, "the blocked address never reaches the app");
    assert!(
        matches!(sent, Err(BeamError::Network { .. })),
        "refused before the handshake: {sent:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unknown_offers_are_rate_limited() {
    let stranger = device();
    let phone = phone(&[]).await;
    for _ in 0..5 {
        // Busy isn't the person saying no, so it doesn't count towards a block.
        Box::pin(phone.offer_and_decline(&stranger, DeclineReason::Busy)).await;
    }
    let (sent, reached) = Box::pin(phone.reaches_the_app(&stranger)).await;
    assert!(!reached, "over the limit, the app isn't bothered");
    assert_eq!(
        sent.unwrap(),
        SendResult::Declined {
            reason: DeclineReason::Busy
        }
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn trusted_devices_are_never_limited() {
    let laptop = device();
    let phone = phone(&[&laptop]).await;
    for _ in 0..6 {
        let trusted = Box::pin(phone.offer_and_decline(&laptop, DeclineReason::UserDeclined)).await;
        assert!(trusted);
    }
}
