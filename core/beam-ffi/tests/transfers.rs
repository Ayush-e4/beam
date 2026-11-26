//! The exported transfer API, driven the way the app drives it: two engines on loopback.

// Test code reports a broken setup by panicking.
#![allow(clippy::unwrap_used, clippy::panic)]

use std::fs;
use std::path::Path;
use std::sync::{Arc, Mutex};

use beam_ffi::{
    BeamError, DeclineReason, DeviceIdentity, Engine, IncomingOffer, Listener, ProgressListener,
    SendResult, SessionCipher, TrustedKeys,
};
use tempfile::TempDir;
use tokio::sync::Notify;

const MIB: usize = 1024 * 1024;
const CIPHER: SessionCipher = SessionCipher::ChaChaPoly;

/// Trusts exactly one key.
struct Trusts(Vec<u8>);

impl TrustedKeys for Trusts {
    fn is_trusted(&self, public_key: Vec<u8>) -> bool {
        public_key == self.0
    }
}

/// Records every report; wakes `passed` once `threshold` bytes are done.
#[derive(Default)]
struct Recorder {
    reports: Mutex<Vec<(u64, u64)>>,
    threshold: u64,
    passed: Notify,
}

impl ProgressListener for Recorder {
    fn on_progress(&self, bytes_done: u64, bytes_total: u64) {
        self.reports.lock().unwrap().push((bytes_done, bytes_total));
        if self.threshold > 0 && bytes_done >= self.threshold {
            self.passed.notify_one();
        }
    }
}

impl Recorder {
    fn watching(threshold: u64) -> Arc<Self> {
        Arc::new(Self {
            threshold,
            ..Self::default()
        })
    }

    fn first(&self) -> (u64, u64) {
        *self.reports.lock().unwrap().first().unwrap()
    }

    fn last(&self) -> (u64, u64) {
        *self.reports.lock().unwrap().last().unwrap()
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

struct Setup {
    _dir: TempDir,
    source: std::path::PathBuf,
    inbox: std::path::PathBuf,
    laptop: Device,
    phone: Device,
    listener: Arc<Listener>,
}

/// A laptop that will send `album` and a phone listening on a free port.
async fn setup(big_file: usize) -> Setup {
    let dir = TempDir::new().unwrap();
    let source = dir.path().join("album");
    let inbox = dir.path().join("inbox");
    fs::create_dir_all(source.join("small")).unwrap();
    fs::create_dir_all(&inbox).unwrap();
    fs::write(source.join("video.bin"), noise(big_file)).unwrap();
    for n in 0..50 {
        fs::write(source.join(format!("small/{n}.txt")), format!("note {n}")).unwrap();
    }
    let (phone, laptop) = (device(), device());
    let trusted = Arc::new(Trusts(laptop.identity.public_key()));
    let listener = phone.engine.listen(0, CIPHER, trusted).await.unwrap();
    Setup {
        _dir: dir,
        source,
        inbox,
        laptop,
        phone,
        listener,
    }
}

impl Setup {
    async fn send(
        &self,
        outgoing: &beam_ffi::OutgoingTransfer,
        progress: Arc<Recorder>,
    ) -> Result<SendResult, BeamError> {
        let port = self.listener.port();
        let key = self.phone.identity.public_key();
        outgoing
            .send_to("127.0.0.1".into(), port, key, CIPHER, progress, None)
            .await
    }

    async fn accept(&self, offer: &IncomingOffer, progress: Arc<Recorder>) -> Vec<String> {
        let inbox = self.inbox.to_string_lossy().into_owned();
        offer.accept(inbox, Vec::new(), progress).await.unwrap()
    }
}

/// Incompressible bytes (xorshift). Compressible data would shrink on the wire, and the
/// receiver checkpoints by wire bytes, so the resume test couldn't count on a checkpoint.
fn noise(len: usize) -> Vec<u8> {
    let mut state = 0x9E37_79B9_7F4A_7C15_u64;
    (0..len)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state.to_le_bytes()[0]
        })
        .collect()
}

fn assert_same(source: &Path, received: &Path) {
    for entry in fs::read_dir(source).unwrap() {
        let entry = entry.unwrap();
        let there = received.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            assert_same(&entry.path(), &there);
        } else {
            assert!(fs::read(entry.path()).unwrap() == fs::read(&there).unwrap());
        }
    }
}

#[tokio::test]
async fn a_folder_arrives_with_progress_on_both_sides() {
    let t = setup(9 * MIB).await;
    let source = t.source.to_string_lossy().into_owned();
    let outgoing = t.laptop.engine.prepare_send(vec![source]).await.unwrap();
    let total = outgoing.summary().total_bytes;
    let (sent, received) = (Recorder::watching(0), Recorder::watching(0));

    let receive = async {
        let offer = t.listener.next_offer().await.unwrap();
        assert_eq!(offer.peer_device_id(), t.laptop.identity.device_id());
        assert_eq!(offer.summary(), outgoing.summary());
        t.accept(&offer, received.clone()).await
    };
    let (result, saved) = tokio::join!(t.send(&outgoing, sent.clone()), receive);
    assert_eq!(result.unwrap(), SendResult::Delivered);
    assert_eq!(saved.len(), 51);
    assert_same(&t.source, &t.inbox.join("album"));
    assert_eq!(sent.last(), (total, total));
    assert_eq!(received.last(), (total, total));
}

#[tokio::test]
async fn a_declined_offer_tells_the_sender_why() {
    let t = setup(MIB).await;
    let source = t.source.to_string_lossy().into_owned();
    let outgoing = t.laptop.engine.prepare_send(vec![source]).await.unwrap();
    let decline = async {
        let offer = t.listener.next_offer().await.unwrap();
        offer.decline(DeclineReason::NoSpace).await.unwrap();
        // Answering twice is refused.
        let again = offer.decline(DeclineReason::Busy).await;
        assert!(
            matches!(again, Err(BeamError::Protocol { .. })),
            "{again:?}"
        );
    };
    let (result, ()) = tokio::join!(t.send(&outgoing, Recorder::watching(0)), decline);
    let declined = SendResult::Declined {
        reason: DeclineReason::NoSpace,
    };
    assert_eq!(result.unwrap(), declined);
}

#[tokio::test]
async fn a_device_with_another_key_is_refused() {
    let t = setup(MIB).await;
    let source = t.source.to_string_lossy().into_owned();
    let outgoing = t.laptop.engine.prepare_send(vec![source]).await.unwrap();
    let stranger = DeviceIdentity::generate().unwrap().public_key();
    let port = t.listener.port();
    // The phone is listening, but the handshake is encrypted to the key the laptop expects:
    // a device with another key can't read it and hangs up, so the laptop sees a dropped link
    // and the phone learns nothing.
    let waiting = tokio::spawn({
        let listener = t.listener.clone();
        async move { listener.next_offer().await }
    });
    let result = outgoing
        .send_to(
            "127.0.0.1".into(),
            port,
            stranger,
            CIPHER,
            Recorder::watching(0),
            None,
        )
        .await;
    assert!(
        matches!(result, Err(BeamError::Network { .. })),
        "{result:?}"
    );
    assert!(!waiting.is_finished(), "the phone keeps listening");
    waiting.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_cancelled_receive_resumes_on_the_next_attempt() {
    let t = setup(96 * MIB).await;
    let source = t.source.to_string_lossy().into_owned();
    let outgoing = t.laptop.engine.prepare_send(vec![source]).await.unwrap();
    let total = outgoing.summary().total_bytes;

    // The phone cancels its receive (as when the app cancels the coroutine) once 16 MiB are in.
    let partial = Recorder::watching(16 * MIB as u64);
    let cancel = async {
        let offer = t.listener.next_offer().await.unwrap();
        tokio::select! {
            _ = t.accept(&offer, partial.clone()) => panic!("finished before the cancel"),
            () = partial.passed.notified() => {}
        }
    };
    let (first, ()) = tokio::join!(t.send(&outgoing, Recorder::watching(0)), cancel);
    assert!(first.is_err(), "{first:?}");

    // The same transfer again: the phone starts from what it kept.
    let resumed = Recorder::watching(0);
    let receive = async {
        let offer = t.listener.next_offer().await.unwrap();
        t.accept(&offer, resumed.clone()).await
    };
    let (second, _) = tokio::join!(t.send(&outgoing, Recorder::watching(0)), receive);
    assert_eq!(second.unwrap(), SendResult::Delivered);
    assert!(resumed.first().0 > 0, "resumed from {:?}", resumed.first());
    assert_eq!(resumed.last(), (total, total));
    assert_same(&t.source, &t.inbox.join("album"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_repeated_offer_for_a_saved_transfer_is_confirmed_not_resent() {
    let t = setup(MIB).await;
    let source = t.source.to_string_lossy().into_owned();
    let outgoing = t.laptop.engine.prepare_send(vec![source]).await.unwrap();
    let receive = async {
        let offer = t.listener.next_offer().await.unwrap();
        t.accept(&offer, Recorder::watching(0)).await
    };
    let (first, _) = tokio::join!(t.send(&outgoing, Recorder::watching(0)), receive);
    assert_eq!(first.unwrap(), SendResult::Delivered);

    // As if `done` was lost: the laptop sends again while the phone waits for new offers.
    let waiting = tokio::spawn({
        let listener = t.listener.clone();
        async move { listener.next_offer().await }
    });
    let again = t.send(&outgoing, Recorder::watching(0)).await;
    assert_eq!(again.unwrap(), SendResult::Delivered);
    assert!(!waiting.is_finished(), "the repeat never reached the app");
    waiting.abort();
    assert_eq!(fs::read_dir(&t.inbox).unwrap().count(), 1, "no duplicate");
}
