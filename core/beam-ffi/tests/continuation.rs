//! Continuing an accepted transfer after the receiving app restarts, and removing ones nobody
//! came back for.

// Test code reports a broken setup by panicking.
#![allow(clippy::unwrap_used, clippy::panic)]

use std::fs;
use std::sync::Arc;

use beam_ffi::{
    DeviceIdentity, Engine, IncomingOffer, ProgressListener, SendResult, SessionCipher,
    TrustedKeys, UnfinishedCleanup,
};
use tempfile::TempDir;
use tokio::sync::Notify;

const MIB: u64 = 1024 * 1024;
const CIPHER: SessionCipher = SessionCipher::ChaChaPoly;
const WEEK_SECS: u64 = 7 * 24 * 60 * 60;

struct TrustsNobody;

impl TrustedKeys for TrustsNobody {
    fn is_trusted(&self, _public_key: Vec<u8>) -> bool {
        false
    }
}

/// Wakes `passed` once `threshold` bytes are done.
struct Watch {
    threshold: u64,
    passed: Notify,
}

impl ProgressListener for Watch {
    fn on_progress(&self, bytes_done: u64, _bytes_total: u64) {
        if self.threshold > 0 && bytes_done >= self.threshold {
            self.passed.notify_one();
        }
    }
}

fn watch(threshold: u64) -> Arc<Watch> {
    Arc::new(Watch {
        threshold,
        passed: Notify::new(),
    })
}

/// Incompressible, so the receiver checkpoints well before the end.
fn noise(len: u64) -> Vec<u8> {
    let mut state = 0x2545_F491_4F6C_DD1D_u64;
    (0..len)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state.to_le_bytes()[0]
        })
        .collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_accepted_transfer_continues_and_stale_ones_are_removed() {
    let dir = TempDir::new().unwrap();
    let source = dir.path().join("clip.bin");
    fs::write(&source, noise(64 * MIB)).unwrap();
    let inbox = dir.path().join("inbox");
    fs::create_dir(&inbox).unwrap();
    let inbox_path = inbox.to_string_lossy().into_owned();

    let laptop = Engine::new(DeviceIdentity::generate().unwrap()).unwrap();
    let phone_identity = DeviceIdentity::generate().unwrap();
    let phone = Engine::new(phone_identity.clone()).unwrap();
    let listener = phone
        .listen(0, CIPHER, Arc::new(TrustsNobody))
        .await
        .unwrap();
    let outgoing = laptop
        .prepare_send(vec![source.to_string_lossy().into_owned()])
        .await
        .unwrap();
    let send = || {
        outgoing.send_to(
            "127.0.0.1".into(),
            listener.port(),
            phone_identity.public_key(),
            CIPHER,
            watch(0),
        )
    };
    let accept = |offer: Arc<IncomingOffer>, progress: Arc<Watch>| {
        let path = inbox_path.clone();
        async move { offer.accept(path, Vec::new(), progress).await }
    };

    // The phone accepts, then the app is closed partway.
    let cut_off = async {
        let offer = listener.next_offer().await.unwrap();
        assert!(
            !offer.continues_in(inbox_path.clone()).await.unwrap(),
            "new"
        );
        let partial = watch(16 * MIB);
        tokio::select! {
            _ = accept(offer, partial.clone()) => panic!("finished before the cut"),
            () = partial.passed.notified() => {}
        }
    };
    let (first, ()) = tokio::join!(send(), cut_off);
    assert!(first.is_err(), "{first:?}");

    // Not stale after a moment.
    let kept = phone
        .remove_stale_transfers(inbox_path.clone(), WEEK_SECS)
        .await
        .unwrap();
    assert_eq!(
        kept,
        UnfinishedCleanup {
            removed: 0,
            failed: 0
        }
    );

    // The laptop tries again: the offer continues what was accepted, so no need to ask.
    let finish = async {
        let offer = listener.next_offer().await.unwrap();
        assert!(offer.continues_in(inbox_path.clone()).await.unwrap());
        let elsewhere = dir.path().to_string_lossy().into_owned();
        assert!(!offer.continues_in(elsewhere).await.unwrap());
        // Stop again, and this time nobody comes back for it.
        let partial = watch(32 * MIB);
        tokio::select! {
            _ = accept(offer, partial.clone()) => panic!("finished before the cut"),
            () = partial.passed.notified() => {}
        }
    };
    let (second, ()) = tokio::join!(send(), finish);
    assert!(second.is_err(), "{second:?}");

    let removed = phone
        .remove_stale_transfers(inbox_path.clone(), 0)
        .await
        .unwrap();
    assert_eq!(
        removed,
        UnfinishedCleanup {
            removed: 1,
            failed: 0
        }
    );
    assert_eq!(
        fs::read_dir(&inbox).unwrap().count(),
        0,
        "nothing left behind"
    );

    // A later attempt starts over and still arrives whole.
    let again = async {
        let offer = listener.next_offer().await.unwrap();
        let path = inbox.to_string_lossy().into_owned();
        assert!(!offer.continues_in(path).await.unwrap());
        accept(offer, watch(0)).await.unwrap()
    };
    let (third, saved) = tokio::join!(send(), again);
    assert_eq!(third.unwrap(), SendResult::Delivered);
    assert_eq!(saved.len(), 1);
    assert!(fs::read(&source).unwrap() == fs::read(inbox.join("clip.bin")).unwrap());
}
