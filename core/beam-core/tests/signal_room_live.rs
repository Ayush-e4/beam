//! The signaling client against the real room service (the Worker in `signal/`, run locally
//! by `wrangler dev`): two devices meet the way the phone and a browser do. Ignored unless
//! `BEAM_SIGNAL_URL` is set; `tools/signal-room-test.sh` starts the Worker and runs it.

// Test code: a failed unwrap or expect is how it reports a broken setup.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use beam_core::{BeamError, RoomEvent, SignalRoom};
use tokio::time::timeout;

const STEP: Duration = Duration::from_secs(10);

fn service() -> String {
    std::env::var("BEAM_SIGNAL_URL").expect("BEAM_SIGNAL_URL, e.g. ws://127.0.0.1:8787")
}

fn random_room() -> [u8; 16] {
    // Only needs to differ between runs and between the tests running at once.
    static TAKEN: AtomicU32 = AtomicU32::new(0);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| since.as_nanos());
    let n = TAKEN.fetch_add(1, Ordering::Relaxed);
    let seed = format!("{nanos} {} {n}", std::process::id());
    let mut room = [0u8; 16];
    room.copy_from_slice(&blake3::hash(seed.as_bytes()).as_bytes()[..16]);
    room
}

async fn next(room: &mut SignalRoom) -> Result<Option<RoomEvent>, BeamError> {
    timeout(STEP, room.next()).await.expect("the room answers")
}

#[tokio::test]
#[ignore = "needs the room service: tools/signal-room-test.sh"]
async fn a_phone_and_a_browser_meet_in_a_room() {
    let service = service();
    let room = random_room();

    let mut phone = SignalRoom::join(&service, &room).await.unwrap();
    assert_eq!(
        next(&mut phone).await.unwrap(),
        Some(RoomEvent::Joined { peers: 0 })
    );
    phone.post(b"sealed offer".to_vec()).await.unwrap();

    // The browser arrives later and still gets the offer the room kept for it.
    let mut browser = SignalRoom::join(&service, &room).await.unwrap();
    assert_eq!(
        next(&mut browser).await.unwrap(),
        Some(RoomEvent::Joined { peers: 1 })
    );
    assert_eq!(
        next(&mut browser).await.unwrap(),
        Some(RoomEvent::Message(b"sealed offer".to_vec()))
    );
    assert_eq!(next(&mut phone).await.unwrap(), Some(RoomEvent::PeerJoined));

    browser.post(b"sealed answer".to_vec()).await.unwrap();
    assert_eq!(
        next(&mut phone).await.unwrap(),
        Some(RoomEvent::Message(b"sealed answer".to_vec()))
    );

    // A third device is turned away: pairings are two devices.
    let mut stranger = SignalRoom::join(&service, &room).await.unwrap();
    match next(&mut stranger).await {
        Err(BeamError::Signaling(why)) => assert!(why.contains("full"), "{why}"),
        other => panic!("a third device got in: {other:?}"),
    }

    browser.leave().await.unwrap();
    assert_eq!(next(&mut phone).await.unwrap(), Some(RoomEvent::PeerLeft));
    phone.leave().await.unwrap();
}

#[tokio::test]
#[ignore = "needs the room service: tools/signal-room-test.sh"]
async fn a_delivered_offer_waits_for_the_browser() {
    let service = service();
    let room = random_room();
    timeout(
        STEP,
        SignalRoom::deliver(&service, &room, b"sealed offer".to_vec()),
    )
    .await
    .expect("the room answers")
    .unwrap();

    // The phone's goodbye may still be on its way, so it may be counted, and leave after.
    let mut browser = SignalRoom::join(&service, &room).await.unwrap();
    assert!(matches!(
        next(&mut browser).await.unwrap(),
        Some(RoomEvent::Joined { peers: 0 | 1 })
    ));
    let mut event = next(&mut browser).await.unwrap();
    if event == Some(RoomEvent::PeerLeft) {
        event = next(&mut browser).await.unwrap();
    }
    assert_eq!(event, Some(RoomEvent::Message(b"sealed offer".to_vec())));
    browser.leave().await.unwrap();
}
