//! A transfer received before the app restarted is confirmed, not offered again, when the
//! sender asks a second time (as when it missed `done`).

// Test code reports a broken setup by panicking.
#![allow(clippy::unwrap_used, clippy::panic)]

use std::fs;
use std::sync::Arc;

use beam_ffi::{
    BeamError, DeviceIdentity, Engine, ProgressListener, SendResult, SessionCipher, TrustedKeys,
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

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_repeat_after_a_restart_is_confirmed_not_offered() {
    let dir = TempDir::new().unwrap();
    let source = dir.path().join("note.txt");
    fs::write(&source, b"arrived once").unwrap();
    let inbox = dir.path().join("inbox");
    fs::create_dir(&inbox).unwrap();
    let list = dir
        .path()
        .join("finished.bin")
        .to_string_lossy()
        .into_owned();

    let laptop = Engine::new(DeviceIdentity::generate().unwrap()).unwrap();
    let outgoing = laptop
        .prepare_send(vec![source.to_string_lossy().into_owned()])
        .await
        .unwrap();
    let phone_identity = DeviceIdentity::generate().unwrap();
    let send = |port: u16| {
        outgoing.send_to(
            "127.0.0.1".into(),
            port,
            phone_identity.public_key(),
            CIPHER,
            Arc::new(Ignore),
        )
    };

    let phone = Engine::new(phone_identity.clone()).unwrap();
    phone.remember_finished_in(list.clone()).await.unwrap();
    let listener = phone
        .listen(0, CIPHER, Arc::new(TrustsNobody))
        .await
        .unwrap();
    let receive = async {
        let offer = listener.next_offer().await.unwrap();
        let path = inbox.to_string_lossy().into_owned();
        offer
            .accept(path, Vec::new(), Arc::new(Ignore))
            .await
            .unwrap()
    };
    let (first, _) = tokio::join!(send(listener.port()), receive);
    assert_eq!(first.unwrap(), SendResult::Delivered);
    drop((listener, phone));

    // The phone's app restarts; the laptop asks again.
    let phone = Engine::new(phone_identity.clone()).unwrap();
    phone.remember_finished_in(list).await.unwrap();
    let listener = phone
        .listen(0, CIPHER, Arc::new(TrustsNobody))
        .await
        .unwrap();
    let waiting = tokio::spawn({
        let listener = listener.clone();
        async move { listener.next_offer().await }
    });
    let again = send(listener.port()).await;
    assert_eq!(again.unwrap(), SendResult::Delivered);
    assert!(!waiting.is_finished(), "the repeat never reached the app");
    waiting.abort();
    assert_eq!(fs::read_dir(&inbox).unwrap().count(), 1, "no duplicate");
}

#[tokio::test]
async fn a_damaged_list_is_reported() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("finished.bin");
    fs::write(&path, b"not a list").unwrap();
    let engine = Engine::new(DeviceIdentity::generate().unwrap()).unwrap();
    let result = engine
        .remember_finished_in(path.to_string_lossy().into_owned())
        .await;
    assert!(
        matches!(result, Err(BeamError::Storage { .. })),
        "{result:?}"
    );
}
