//! Pairing two engines on loopback, then using what pairing returned to send a file.

// Test code reports a broken setup by panicking.
#![allow(clippy::unwrap_used, clippy::panic)]

use std::net::SocketAddr;
use std::sync::Arc;

use beam_core::{Beacon, DiscoveryKey, PairingInvite};
use beam_ffi::{
    BeamError, DeviceIdentity, Engine, PairingHost, ProgressListener, SendResult, SessionCipher,
};

struct Phone {
    identity: Arc<DeviceIdentity>,
    engine: Arc<Engine>,
}

fn phone() -> Phone {
    let identity = DeviceIdentity::generate().unwrap();
    Phone {
        engine: Engine::new(identity.clone()).unwrap(),
        identity,
    }
}

async fn host(phone: &Phone) -> (Arc<PairingHost>, String) {
    let host = phone
        .engine
        .host_pairing(SessionCipher::AesGcm)
        .await
        .unwrap();
    let code = host.code(vec!["127.0.0.1".into()]).unwrap();
    (host, code)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pairing_hands_over_discovery_keys_for_the_current_generation() {
    let (a, b) = (phone(), phone());
    a.engine.set_discovery_generation(2);
    let (host, code) = host(&a).await;
    let (shown, scanned) = tokio::join!(
        host.wait_for_device("Phone A".into()),
        b.engine.pair_with(code, "Phone B".into())
    );
    let (shown, scanned) = (shown.unwrap(), scanned.unwrap());

    let a_key = DiscoveryKey::derive(a.identity.keypair(), 2);
    let b_key = DiscoveryKey::derive(b.identity.keypair(), 0);
    assert_eq!(scanned.discovery_key, a_key.as_bytes());
    assert_eq!(shown.discovery_key, b_key.as_bytes());

    // What B stored is enough to recognise A's beacons, and only A's.
    let stored = DiscoveryKey::from_slice(&scanned.discovery_key).unwrap();
    assert!(Beacon::new(&a_key).unwrap().matches(&stored));
    assert!(!Beacon::new(&b_key).unwrap().matches(&stored));
}

/// A copy of `code` with one field changed.
fn altered(code: &str, change: impl FnOnce(&mut PairingInvite)) -> String {
    let mut invite = PairingInvite::decode(code).unwrap();
    change(&mut invite);
    invite.encode()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn two_phones_pair_and_learn_each_other() {
    let (a, b) = (phone(), phone());
    let (host, code) = host(&a).await;
    let (shown, scanned) = tokio::join!(
        host.wait_for_device("Phone A".into()),
        b.engine.pair_with(code, "Phone B".into())
    );
    let (shown, scanned) = (shown.unwrap(), scanned.unwrap());

    assert_eq!(shown.public_key, b.identity.public_key());
    assert_eq!(shown.device_id, b.identity.device_id());
    assert_eq!(shown.name, "Phone B");
    assert_eq!(scanned.public_key, a.identity.public_key());
    assert_eq!(scanned.name, "Phone A");
    assert_eq!(shown.cipher, SessionCipher::AesGcm);
    assert_eq!(scanned.cipher, SessionCipher::AesGcm);
    assert_eq!(shown.verification_code, scanned.verification_code);
    assert_eq!(shown.verification_code.len(), 7, "like 042 917");

    // The code is spent.
    let again = host.wait_for_device("Phone A".into()).await;
    assert!(
        matches!(again, Err(BeamError::Protocol { .. })),
        "{again:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn forged_codes_fail_and_the_host_keeps_waiting() {
    let (a, b, mallory) = (phone(), phone(), phone());
    let (host, code) = host(&a).await;
    let waiting = tokio::spawn({
        let host = host.clone();
        async move { host.wait_for_device("Phone A".into()).await }
    });

    // An old or guessed token, and a code claiming another device's key, both fail.
    let wrong_token = altered(&code, |invite| invite.token[0] ^= 1);
    let wrong_key = altered(&code, |invite| {
        invite.displayer =
            beam_core::PublicKey::from_bytes(mallory.identity.public_key().try_into().unwrap());
    });
    for forged in [wrong_token, wrong_key] {
        let result = b.engine.pair_with(forged, "Phone B".into()).await;
        assert!(
            matches!(
                result,
                Err(BeamError::Crypto { .. } | BeamError::Network { .. })
            ),
            "{result:?}"
        );
        assert!(!waiting.is_finished(), "the host keeps waiting");
    }

    // The real code still works.
    let paired = b.engine.pair_with(code, "Phone B".into()).await.unwrap();
    assert_eq!(paired.public_key, a.identity.public_key());
    assert_eq!(waiting.await.unwrap().unwrap().name, "Phone B");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_unreachable_address_does_not_stop_pairing() {
    let (a, b) = (phone(), phone());
    let (host, code) = host(&a).await;
    let closed = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let dead: SocketAddr = closed.local_addr().unwrap();
    drop(closed);
    let code = altered(&code, |invite| {
        let live = invite.hints[0];
        *invite = invite.with_hints(vec![dead, live]).unwrap();
    });
    let (shown, scanned) = tokio::join!(
        host.wait_for_device("Phone A".into()),
        b.engine.pair_with(code, "Phone B".into())
    );
    assert_eq!(shown.unwrap().name, "Phone B");
    assert_eq!(scanned.unwrap().name, "Phone A");
}

#[tokio::test]
async fn other_codes_are_refused_with_a_reason() {
    let b = phone();
    for (code, why) in [
        ("https://example.com", "not a Beam pairing code"),
        ("BEAM1:AAAA", "pairing code"),
    ] {
        let result = b.engine.pair_with(code.into(), "Phone B".into()).await;
        match result {
            Err(BeamError::Protocol { detail }) => assert!(detail.contains(why), "{detail}"),
            other => panic!("{code}: {other:?}"),
        }
    }
}

struct Quiet;

impl ProgressListener for Quiet {
    fn on_progress(&self, _: u64, _: u64) {}
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_paired_device_can_send_right_away() {
    let (a, b) = (phone(), phone());
    let (host, code) = host(&a).await;
    let (_, scanned) = tokio::join!(
        host.wait_for_device("Phone A".into()),
        b.engine.pair_with(code, "Phone B".into())
    );
    let phone_a = scanned.unwrap();

    // B sends to A using only what pairing gave it: A's key and the cipher.
    let dir = tempfile::TempDir::new().unwrap();
    let note = dir.path().join("hello.txt");
    std::fs::write(&note, b"hi from B").unwrap();
    let inbox = dir.path().join("inbox");
    std::fs::create_dir(&inbox).unwrap();
    let listener = a.engine.listen(0, phone_a.cipher).await.unwrap();
    let outgoing = b
        .engine
        .prepare_send(vec![note.to_string_lossy().into_owned()])
        .await
        .unwrap();
    let receive = async {
        let offer = listener.next_offer().await.unwrap();
        assert_eq!(offer.peer_device_id(), b.identity.device_id());
        let inbox = inbox.to_string_lossy().into_owned();
        offer
            .accept(inbox, Vec::new(), Arc::new(Quiet))
            .await
            .unwrap()
    };
    let port = listener.port();
    let send = outgoing.send_to(
        "127.0.0.1".into(),
        port,
        phone_a.public_key.clone(),
        phone_a.cipher,
        Arc::new(Quiet),
    );
    let (outcome, _) = tokio::join!(send, receive);
    assert_eq!(outcome.unwrap(), SendResult::Delivered);
    assert_eq!(
        std::fs::read(inbox.join("hello.txt")).unwrap(),
        b"hi from B"
    );
}
