//! A send cut off when the app was closed, prepared again under its old transfer ID by a
//! fresh engine (as after the app restarts): the receiver continues it.

// Test code reports a broken setup by panicking.
#![allow(clippy::unwrap_used, clippy::panic)]

use std::fs::{self, File};
use std::sync::{Arc, Mutex};

use beam_ffi::{
    BeamError, DeviceIdentity, Engine, ProgressListener, SendResult, SessionCipher, SharedFile,
    TrustedKeys,
};
use tempfile::TempDir;
use tokio::sync::Notify;

const MIB: u64 = 1024 * 1024;
const CIPHER: SessionCipher = SessionCipher::ChaChaPoly;

struct TrustsNobody;

impl TrustedKeys for TrustsNobody {
    fn is_trusted(&self, _public_key: Vec<u8>) -> bool {
        false
    }
}

/// Keeps the first report; wakes `passed` once `threshold` bytes are done.
#[derive(Default)]
struct Watch {
    threshold: u64,
    first: Mutex<Option<u64>>,
    passed: Notify,
}

impl ProgressListener for Watch {
    fn on_progress(&self, bytes_done: u64, _bytes_total: u64) {
        self.first.lock().unwrap().get_or_insert(bytes_done);
        if self.threshold > 0 && bytes_done >= self.threshold {
            self.passed.notify_one();
        }
    }
}

fn watch(threshold: u64) -> Arc<Watch> {
    Arc::new(Watch {
        threshold,
        ..Watch::default()
    })
}

fn noise(len: u64) -> Vec<u8> {
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

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_send_resumes_under_its_old_id_after_a_restart() {
    let dir = TempDir::new().unwrap();
    let source = dir.path().join("clip.bin");
    fs::write(&source, noise(64 * MIB)).unwrap();
    let inbox = dir.path().join("inbox");
    fs::create_dir(&inbox).unwrap();
    let inbox_path = inbox.to_string_lossy().into_owned();
    let paths = vec![source.to_string_lossy().into_owned()];

    let phone_identity = DeviceIdentity::generate().unwrap();
    let phone = Engine::new(phone_identity.clone()).unwrap();
    let listener = phone
        .listen(0, CIPHER, Arc::new(TrustsNobody))
        .await
        .unwrap();
    let laptop_identity = DeviceIdentity::generate().unwrap();
    let send = |outgoing: Arc<beam_ffi::OutgoingTransfer>| {
        let key = phone_identity.public_key();
        let port = listener.port();
        async move {
            outgoing
                .send_to("127.0.0.1".into(), port, key, CIPHER, watch(0))
                .await
        }
    };

    // The laptop's app starts the send and is closed partway.
    let first_engine = Engine::new(laptop_identity.clone()).unwrap();
    let first = first_engine.prepare_send(paths.clone()).await.unwrap();
    let transfer_id = first.summary().transfer_id;
    let cut_off = async {
        let offer = listener.next_offer().await.unwrap();
        let partial = watch(16 * MIB);
        tokio::select! {
            _ = offer.accept(inbox_path.clone(), Vec::new(), partial.clone()) => {
                panic!("finished before the cut")
            }
            () = partial.passed.notified() => {}
        }
    };
    let (result, ()) = tokio::join!(send(first.clone()), cut_off);
    assert!(result.is_err(), "{result:?}");
    drop((first, first_engine));

    // It starts again and resumes the remembered send.
    let engine = Engine::new(laptop_identity).unwrap();
    let resumed = engine
        .resume_send(paths, transfer_id.clone())
        .await
        .unwrap();
    assert_eq!(resumed.summary().transfer_id, transfer_id);
    let receive = async {
        let offer = listener.next_offer().await.unwrap();
        assert!(offer.continues_in(inbox_path.clone()).await.unwrap());
        let progress = watch(0);
        offer
            .accept(inbox_path.clone(), Vec::new(), progress.clone())
            .await
            .unwrap();
        progress.first.lock().unwrap().unwrap_or(0)
    };
    let (result, started_at) = tokio::join!(send(resumed), receive);
    assert_eq!(result.unwrap(), SendResult::Delivered);
    assert!(
        started_at > 0,
        "continued from {started_at}, not from the start"
    );
    assert!(fs::read(&source).unwrap() == fs::read(inbox.join("clip.bin")).unwrap());
}

#[tokio::test]
async fn shared_files_resume_under_their_old_id_too() {
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("a"), b"photo").unwrap();
    let file = File::open(dir.path().join("a")).unwrap();
    let shared = || {
        use std::os::fd::AsRawFd;
        vec![SharedFile {
            name: "IMG_0042.jpg".into(),
            fd: file.as_raw_fd(),
        }]
    };
    let engine = Engine::new(DeviceIdentity::generate().unwrap()).unwrap();
    let id = engine
        .prepare_send_shared(shared())
        .await
        .unwrap()
        .summary()
        .transfer_id;
    let again = engine
        .resume_send_shared(shared(), id.clone())
        .await
        .unwrap();
    assert_eq!(again.summary().transfer_id, id);
}

#[tokio::test]
async fn anything_but_a_transfer_id_is_refused() {
    let engine = Engine::new(DeviceIdentity::generate().unwrap()).unwrap();
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("a");
    fs::write(&path, b"x").unwrap();
    let paths = vec![path.to_string_lossy().into_owned()];
    for id in ["", "abc", &"g".repeat(32), &"0".repeat(31), &"é".repeat(16)] {
        let result = engine.resume_send(paths.clone(), id.to_owned()).await;
        assert!(
            matches!(result, Err(BeamError::Internal { .. })),
            "{id:?}: {result:?}"
        );
    }
}
