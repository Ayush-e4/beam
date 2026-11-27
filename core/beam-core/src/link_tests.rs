use std::time::Duration;

use tokio::io::{DuplexStream, duplex};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use super::*;
use crate::ErrorCategory;
use crate::handshake::{Cipher, Handshake, Role};
use crate::keys::StaticKeypair;

type Link = Connection<DuplexStream>;

const TOKEN: &[u8; 16] = b"one-time-token!!";
const FAST: LinkTiming = LinkTiming {
    ping_every: Duration::from_millis(40),
    dead_after: Duration::from_millis(150),
};

async fn connected() -> (Link, Link) {
    let (scanner, phone) = (
        StaticKeypair::generate().unwrap(),
        StaticKeypair::generate().unwrap(),
    );
    let (a, b) = duplex(256 * 1024);
    let i = Handshake::pairing_scanner(
        &scanner,
        Cipher::AesGcm,
        Role::Initiator,
        TOKEN,
        phone.public(),
    )
    .unwrap();
    let r = Handshake::pairing_displayer(&phone, Cipher::AesGcm, Role::Responder, TOKEN).unwrap();
    let (a, b) = tokio::join!(Connection::establish(a, i), Connection::establish(b, r));
    (a.unwrap(), b.unwrap())
}

/// A running link: where to put items to send, where items arrive, and how it ended.
struct Side {
    send: mpsc::Sender<ClipItem>,
    arrived: mpsc::Receiver<ClipItem>,
    ended: JoinHandle<Result<LinkEnd>>,
}

fn start(mut conn: Link, timing: LinkTiming) -> Side {
    let (send, outgoing) = mpsc::channel(4);
    let (incoming, arrived) = mpsc::channel(4);
    let ended = tokio::spawn(async move { run_link(&mut conn, outgoing, incoming, timing).await });
    Side {
        send,
        arrived,
        ended,
    }
}

fn text(text: &str) -> ClipItem {
    ClipItem::text(text).unwrap()
}

#[tokio::test]
async fn items_cross_both_ways() {
    let (a, b) = connected().await;
    let (mut a, mut b) = (start(a, FAST), start(b, FAST));
    a.send.send(text("from the phone")).await.unwrap();
    let image = ClipItem::image("image/png", vec![3; 64 * 1024]).unwrap();
    b.send.send(image.clone()).await.unwrap();
    b.send.send(text("482913")).await.unwrap();
    assert_eq!(b.arrived.recv().await.unwrap(), text("from the phone"));
    assert_eq!(a.arrived.recv().await.unwrap(), image);
    assert_eq!(a.arrived.recv().await.unwrap(), text("482913"));
}

#[tokio::test]
async fn stopping_one_side_ends_both_cleanly() {
    let (a, b) = connected().await;
    let (a, b) = (start(a, FAST), start(b, FAST));
    drop(a.send);
    assert_eq!(a.ended.await.unwrap().unwrap(), LinkEnd::Stopped);
    assert_eq!(b.ended.await.unwrap().unwrap(), LinkEnd::PeerClosed);
}

#[tokio::test]
async fn pings_keep_a_quiet_link_alive() {
    let (a, b) = connected().await;
    let (a, mut b) = (start(a, FAST), start(b, FAST));
    tokio::time::sleep(FAST.dead_after * 4).await;
    assert!(!a.ended.is_finished() && !b.ended.is_finished());
    a.send.send(text("still here")).await.unwrap();
    assert_eq!(b.arrived.recv().await.unwrap(), text("still here"));
}

#[tokio::test]
async fn a_silent_peer_ends_the_link() {
    let (a, _silent) = connected().await;
    let a = start(a, FAST);
    let err = tokio::time::timeout(FAST.dead_after * 4, a.ended)
        .await
        .expect("the link gives up on its own")
        .unwrap()
        .unwrap_err();
    assert_eq!(err.category(), ErrorCategory::Network);
}

#[tokio::test]
async fn frames_that_dont_belong_end_the_link() {
    for frame in [
        Frame::new(FrameType::Offer, 1, vec![1, 2, 3]).unwrap(),
        Frame::new(FrameType::Clipboard, 0, vec![9, 0, 0]).unwrap(),
    ] {
        let (a, mut raw) = connected().await;
        let a = start(a, FAST);
        raw.send(&frame).await.unwrap();
        let err = a.ended.await.unwrap().unwrap_err();
        assert!(matches!(err, BeamError::Malformed(_)), "{frame:?}: {err}");
    }
}

#[tokio::test]
async fn the_link_stops_when_nobody_listens_for_items() {
    let (a, b) = connected().await;
    let (a, b) = (start(a, FAST), start(b, FAST));
    drop(b.arrived);
    a.send.send(text("anyone?")).await.unwrap();
    assert_eq!(b.ended.await.unwrap().unwrap(), LinkEnd::Stopped);
    assert_eq!(a.ended.await.unwrap().unwrap(), LinkEnd::PeerClosed);
}
