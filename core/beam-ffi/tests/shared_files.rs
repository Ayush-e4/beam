//! Files the app shares by file descriptor, as Android hands over gallery and share-sheet items.

// Test code reports a broken setup by panicking.
#![allow(clippy::unwrap_used, clippy::panic)]
#![cfg(unix)]

use std::fs::{self, File};
use std::io::Read;
use std::os::fd::AsRawFd;
use std::sync::Arc;

use beam_ffi::{
    BeamError, DeviceIdentity, Engine, ProgressListener, SendResult, SessionCipher, SharedFile,
    TrustedKeys,
};
use tempfile::TempDir;

const CIPHER: SessionCipher = SessionCipher::ChaChaPoly;

struct TrustsNobody;

impl TrustedKeys for TrustsNobody {
    fn is_trusted(&self, _public_key: Vec<u8>) -> bool {
        false
    }
}

struct Ignore;

impl ProgressListener for Ignore {
    fn on_progress(&self, _bytes_done: u64, _bytes_total: u64) {}
}

fn engine() -> (Arc<DeviceIdentity>, Arc<Engine>) {
    let identity = DeviceIdentity::generate().unwrap();
    (identity.clone(), Engine::new(identity).unwrap())
}

fn shared(name: &str, file: &File) -> SharedFile {
    SharedFile {
        name: name.into(),
        fd: file.as_raw_fd(),
    }
}

#[tokio::test]
async fn shared_files_arrive_and_the_app_keeps_its_descriptors() {
    let dir = TempDir::new().unwrap();
    let photo: Vec<u8> = (0..3_000_000u32).map(|n| (n % 253) as u8).collect();
    fs::write(dir.path().join("a"), &photo).unwrap();
    fs::write(dir.path().join("b"), b"a short note").unwrap();
    let a = File::open(dir.path().join("a")).unwrap();
    let b = File::open(dir.path().join("b")).unwrap();
    let files = vec![shared("IMG_0042.jpg", &a), shared("note.txt", &b)];

    let (_, laptop) = engine();
    let outgoing = laptop.prepare_send_shared(files).await.unwrap();
    // The app closes its descriptors and the files leave the disk: the engine still has them.
    drop((a, b));
    fs::remove_file(dir.path().join("a")).unwrap();
    fs::remove_file(dir.path().join("b")).unwrap();

    let (phone_identity, phone) = engine();
    let listener = phone
        .listen(0, CIPHER, Arc::new(TrustsNobody))
        .await
        .unwrap();
    let inbox = dir.path().join("inbox");
    fs::create_dir(&inbox).unwrap();
    let receive = async {
        let offer = listener.next_offer().await.unwrap();
        let path = inbox.to_string_lossy().into_owned();
        offer
            .accept(path, Vec::new(), Arc::new(Ignore))
            .await
            .unwrap()
    };
    let send = outgoing.send_to(
        "127.0.0.1".into(),
        listener.port(),
        phone_identity.public_key(),
        CIPHER,
        Arc::new(Ignore),
    );
    let (result, saved) = tokio::join!(send, receive);
    assert_eq!(result.unwrap(), SendResult::Delivered);
    assert_eq!(saved.len(), 2);
    assert_eq!(fs::read(inbox.join("IMG_0042.jpg")).unwrap(), photo);
    assert_eq!(fs::read(inbox.join("note.txt")).unwrap(), b"a short note");
}

#[tokio::test]
async fn the_engine_never_closes_the_apps_descriptor() {
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("a"), b"still mine").unwrap();
    let mut file = File::open(dir.path().join("a")).unwrap();
    let (_, laptop) = engine();
    let outgoing = laptop
        .prepare_send_shared(vec![shared("a.txt", &file)])
        .await
        .unwrap();
    drop(outgoing);

    let mut text = String::new();
    file.read_to_string(&mut text).unwrap();
    assert_eq!(text, "still mine");
}

#[tokio::test]
async fn descriptors_that_are_not_open_files_are_refused() {
    let dir = TempDir::new().unwrap();
    let folder = File::open(dir.path()).unwrap();
    let (_, laptop) = engine();
    for fd in [-1, 1_000_000, folder.as_raw_fd()] {
        let files = vec![SharedFile {
            name: "x".into(),
            fd,
        }];
        let result = laptop.prepare_send_shared(files).await;
        assert!(
            matches!(result, Err(BeamError::Storage { .. })),
            "fd {fd}: {result:?}"
        );
    }
}
