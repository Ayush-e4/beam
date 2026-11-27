//! The phone's side of pairing with a browser, short of a browser connecting (that runs in
//! Chromium: `core/beam-web/e2e/phone-pairing.e2e.mjs`): the offer it posts, the room it posts
//! it in, and the codes it refuses.

// Test code reports a broken setup by panicking.
#![allow(clippy::unwrap_used, clippy::panic)]

use std::fmt::Write as _;
use std::net::SocketAddr;
use std::sync::Arc;

use beam_core::{Cipher, PairingInvite, SignalFrom, SignalMessage, SignalSeal, StaticKeypair};
use beam_ffi::{BeamError, BrowserPairing, DeviceIdentity, Engine};

/// What a browser shows: an invite with no addresses, since it can't accept connections.
fn browser_code() -> (PairingInvite, String) {
    let browser = StaticKeypair::generate().unwrap();
    let invite = PairingInvite::new(browser.public(), Cipher::ChaChaPoly, Vec::new()).unwrap();
    let code = invite.encode();
    (invite, code)
}

async fn scanned(code: String) -> (Arc<Engine>, Arc<BrowserPairing>) {
    let engine = Engine::new(DeviceIdentity::generate().unwrap()).unwrap();
    let pairing = engine.pair_with_browser(code).await.unwrap();
    (engine, pairing)
}

/// Opens `sealed` as the browser showing `invite` does.
fn open(invite: &PairingInvite, sealed: &[u8]) -> (Vec<SocketAddr>, [u8; 32]) {
    let seal = SignalSeal::for_invite(invite).unwrap();
    let SignalMessage::Offer {
        hints,
        certificate_hash,
    } = seal.open(SignalFrom::Scanner, sealed).unwrap();
    (hints, certificate_hash)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_offer_tells_the_browser_where_to_connect() {
    let (invite, code) = browser_code();
    let (_engine, pairing) = scanned(code).await;
    let port = pairing.port().unwrap();
    let addresses = ["192.168.1.20", "fe80::1", "2001:db8::7"].map(String::from);
    let (hints, hash) = open(&invite, &pairing.offer(addresses.to_vec()).unwrap());
    let expected: Vec<SocketAddr> = [
        format!("192.168.1.20:{port}"),
        format!("[2001:db8::7]:{port}"),
    ]
    .iter()
    .map(|hint| hint.parse().unwrap())
    .collect();
    assert_eq!(hints, expected, "link-local IPv6 is skipped");
    assert_ne!(hash, [0; 32], "the certificate's hash, to pin");
    let (_, again) = open(&invite, &pairing.offer(vec!["10.0.0.2".into()]).unwrap());
    assert_eq!(again, hash, "one server, one certificate");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_offer_goes_in_the_codes_room() {
    let (invite, code) = browser_code();
    let (_engine, pairing) = scanned(code).await;
    let room = invite.room().iter().fold(String::new(), |mut out, b| {
        write!(out, "{b:02x}").unwrap();
        out
    });
    assert_eq!(pairing.room(), room);
    assert_eq!(pairing.room().len(), 32);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_offer_needs_a_usable_address() {
    let (_, code) = browser_code();
    let (_engine, pairing) = scanned(code).await;
    for addresses in [
        vec![],
        vec!["fe80::1".to_owned()],
        vec!["phone.local".to_owned()],
    ] {
        let refused = pairing.offer(addresses.clone());
        assert!(
            matches!(refused, Err(BeamError::Protocol { .. })),
            "{addresses:?}: {refused:?}"
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_code_that_isnt_beams_is_refused() {
    let engine = Engine::new(DeviceIdentity::generate().unwrap()).unwrap();
    let refused = engine.pair_with_browser("https://example.com".into()).await;
    assert!(
        matches!(refused, Err(BeamError::Protocol { .. })),
        "{refused:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stopping_ends_the_wait() {
    let (_, code) = browser_code();
    let (_engine, pairing) = scanned(code).await;
    let waiting = {
        let pairing = pairing.clone();
        tokio::spawn(async move { pairing.wait_for_browser("Phone".into()).await })
    };
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    pairing.stop();
    let ended = tokio::time::timeout(std::time::Duration::from_secs(5), waiting)
        .await
        .expect("stopping ends the wait")
        .unwrap();
    assert!(matches!(ended, Err(BeamError::Network { .. })), "{ended:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn announcing_needs_a_reachable_service_and_an_address() {
    let (_, code) = browser_code();
    let (_engine, pairing) = scanned(code).await;
    let unused = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap();
    let unreachable = pairing
        .announce(format!("ws://{unused}"), vec!["10.0.0.2".into()])
        .await;
    assert!(
        matches!(unreachable, Err(BeamError::Network { .. })),
        "{unreachable:?}"
    );
    let no_address = pairing.announce(format!("ws://{unused}"), vec![]).await;
    assert!(
        matches!(no_address, Err(BeamError::Protocol { .. })),
        "{no_address:?}"
    );
}

/// Against the real room service (`tools/signal-room-test.sh` sets `BEAM_SIGNAL_URL`): the
/// browser showing the code finds the phone's offer in its room.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs the room service: tools/signal-room-test.sh"]
async fn the_browser_finds_the_announced_offer_in_its_room() {
    let service = std::env::var("BEAM_SIGNAL_URL").unwrap();
    let (invite, code) = browser_code();
    let (_engine, pairing) = scanned(code).await;
    pairing
        .announce(service.clone(), vec!["192.168.1.20".into()])
        .await
        .unwrap();

    let mut browser = beam_core::SignalRoom::join(&service, &invite.room())
        .await
        .unwrap();
    let sealed = loop {
        match browser.next().await.unwrap() {
            Some(beam_core::RoomEvent::Message(sealed)) => break sealed,
            Some(_) => {}
            None => panic!("the room closed before the offer"),
        }
    };
    let (hints, _) = open(&invite, &sealed);
    let port = pairing.port().unwrap();
    assert_eq!(hints, vec![format!("192.168.1.20:{port}").parse().unwrap()]);
    browser.leave().await.unwrap();
}
