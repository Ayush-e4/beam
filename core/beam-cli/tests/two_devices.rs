//! Two CLI devices in one process: pair, then send and receive over loopback.

// Test code reports a broken setup by panicking.
#![allow(clippy::unwrap_used, clippy::panic)]

use std::fs;
use std::time::Duration;

use beam_cli::commands::{self, Out};
use beam_cli::home::Home;
use beam_ffi::{PairedDevice, SessionCipher};
use tempfile::TempDir;
use tokio::sync::mpsc::{self, UnboundedReceiver};
use tokio::time::timeout;

const CIPHER: SessionCipher = SessionCipher::AesGcm;

fn channel() -> (Out, UnboundedReceiver<String>) {
    mpsc::unbounded_channel()
}

/// The next line starting with `word`.
async fn next(lines: &mut UnboundedReceiver<String>, word: &str) -> String {
    timeout(Duration::from_secs(20), async {
        loop {
            let line = lines.recv().await.unwrap();
            if line.starts_with(word) {
                return line;
            }
        }
    })
    .await
    .unwrap_or_else(|_| panic!("no {word} line"))
}

fn field<'a>(line: &'a str, key: &str) -> &'a str {
    line.split_whitespace()
        .find_map(|pair| pair.strip_prefix(&format!("{key}=")))
        .unwrap_or_else(|| panic!("no {key} in {line}"))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn paired_devices_send_and_strangers_are_declined() {
    let dir = TempDir::new().unwrap();
    let mac = Home::at(dir.path().join("mac"));
    let phone = Home::at(dir.path().join("phone"));

    // The Mac shows a code; the phone pairs with it.
    let (out, mut mac_lines) = channel();
    let host = {
        let mac = mac.clone();
        tokio::spawn(async move {
            let address = vec!["127.0.0.1".to_owned()];
            commands::pair(&mac, "Mac".into(), address, CIPHER, &out).await
        })
    };
    let code = next(&mut mac_lines, "CODE ").await["CODE ".len()..].to_owned();
    let (out, mut phone_lines) = channel();
    commands::pair_with(&phone, code, "Phone".into(), &out)
        .await
        .unwrap();
    host.await.unwrap().unwrap();
    let mac_saw = next(&mut mac_lines, "PAIRED").await;
    let phone_saw = next(&mut phone_lines, "PAIRED").await;
    assert_eq!(field(&mac_saw, "verify"), field(&phone_saw, "verify"));
    assert_eq!(phone.find("Mac").unwrap().cipher, CIPHER);

    // The Mac receives; the phone sends to it.
    let inbox = dir.path().join("inbox");
    let (out, mut mac_lines) = channel();
    let receiving = {
        let (mac, inbox) = (mac.clone(), inbox.clone());
        tokio::spawn(async move { commands::receive(&mac, 0, inbox, false, CIPHER, &out).await })
    };
    let port: u16 = field(&next(&mut mac_lines, "LISTENING").await, "port")
        .parse()
        .unwrap();
    let file = dir.path().join("photo.jpg");
    fs::write(&file, vec![42u8; 300_000]).unwrap();
    let files = vec![file.to_string_lossy().into_owned()];
    let (out, mut phone_lines) = channel();
    commands::send(&phone, "Mac", "127.0.0.1".into(), port, files.clone(), &out)
        .await
        .unwrap();
    let sent = next(&mut phone_lines, "SENT").await;
    let received = next(&mut mac_lines, "RECEIVED").await;
    assert_eq!(field(&sent, "transfer"), field(&received, "transfer"));
    assert_eq!(
        fs::read(inbox.join("photo.jpg")).unwrap(),
        vec![42u8; 300_000]
    );

    // A device the Mac never paired with is declined.
    let stranger = Home::at(dir.path().join("stranger"));
    let mac_key = mac.identity().unwrap();
    stranger
        .trust(&PairedDevice {
            public_key: mac_key.public_key(),
            device_id: mac_key.device_id(),
            name: "Mac".into(),
            cipher: CIPHER,
            verification_code: String::new(),
            discovery_key: vec![0; 32],
        })
        .unwrap();
    let (out, mut stranger_lines) = channel();
    commands::send(&stranger, "Mac", "127.0.0.1".into(), port, files, &out)
        .await
        .unwrap();
    assert!(
        next(&mut stranger_lines, "DECLINED")
            .await
            .contains("reason=UserDeclined")
    );
    next(&mut mac_lines, "DECLINED").await;
    receiving.abort();
}
