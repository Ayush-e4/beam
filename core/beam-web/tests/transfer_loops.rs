//! The transfer loops in a real browser worker: a phone-side `send_transfer` and a browser-side
//! `receive_transfer` run over an encrypted connection (an in-memory pipe stands in for
//! WebTransport) and the files land in OPFS. The engine's timers and clock are the browser's
//! here (`setTimeout`, `performance.now()`), not tokio's.
//!
//! Run with `tools/web-engine-test.sh`.
#![cfg(target_family = "wasm")]
#![allow(clippy::unwrap_used)]

mod common;

use std::future::{Future, poll_fn};
use std::pin::pin;
use std::sync::{Arc, Mutex};
use std::task::Poll;

use beam_core::{
    Cipher, Connection, Handshake, Receiver, SendOutcome, Sender, StaticKeypair, prepare_readable,
    read_offer, receive_transfer, send_transfer,
};
use beam_web::opfs_staging::{OpfsStaging, remove_staged};
use common::{assert_intact, readable, root};
use tokio::io::duplex;
use wasm_bindgen_test::{wasm_bindgen_test, wasm_bindgen_test_configure};

wasm_bindgen_test_configure!(run_in_dedicated_worker);

const PIPE_CAPACITY: usize = 256 * 1024;

/// Runs both futures to completion on this one thread.
async fn both<A: Future, B: Future>(a: A, b: B) -> (A::Output, B::Output) {
    let (mut a, mut b) = (pin!(a), pin!(b));
    let (mut a_out, mut b_out) = (None, None);
    poll_fn(|cx| {
        if a_out.is_none() {
            a_out = match a.as_mut().poll(cx) {
                Poll::Ready(out) => Some(out),
                Poll::Pending => None,
            };
        }
        if b_out.is_none() {
            b_out = match b.as_mut().poll(cx) {
                Poll::Ready(out) => Some(out),
                Poll::Pending => None,
            };
        }
        if a_out.is_some() && b_out.is_some() {
            Poll::Ready((a_out.take().unwrap(), b_out.take().unwrap()))
        } else {
            Poll::Pending
        }
    })
    .await
}

/// Sends the test files from a "phone" to this worker, keeping at most `window` bytes in
/// flight if given, and checks they arrive intact in OPFS.
async fn phone_to_browser(window: Option<u64>) {
    let prepared = prepare_readable(readable()).unwrap();
    let manifest = prepared.manifest().clone();
    let (phone_end, browser_end) = duplex(PIPE_CAPACITY);
    let phone = StaticKeypair::generate().unwrap();
    let browser = StaticKeypair::generate().unwrap();
    let browser_key = browser.public();

    let sending = async {
        let handshake = Handshake::known_initiator(&phone, Cipher::ChaChaPoly, browser_key);
        let mut conn = Connection::establish(phone_end, handshake.unwrap())
            .await
            .unwrap();
        let sender = match window {
            Some(window) => Sender::with_window(prepared, window),
            None => Sender::new(prepared),
        };
        // `send_transfer` shares the sender with its file work; here there's one thread.
        #[allow(clippy::arc_with_non_send_sync)]
        let sender = Arc::new(Mutex::new(sender));
        send_transfer(&mut conn, &sender).await
    };
    let receiving = async {
        let handshake = Handshake::known_responder(&browser, Cipher::ChaChaPoly).unwrap();
        let mut conn = Connection::establish(browser_end, handshake).await.unwrap();
        let offered = read_offer(&mut conn).await.unwrap();
        let sender_key = conn.peer().as_bytes().to_vec();
        let staging = OpfsStaging::open(&root().await, &offered, &[], &sender_key).await;
        let receiver = Receiver::open_staged(offered, &[], Box::new(staging.unwrap())).unwrap();
        (receive_transfer(&mut conn, receiver).await, sender_key)
    };
    let (sent, (received, sender_key)) = both(sending, receiving).await;
    assert_eq!(sent.unwrap(), SendOutcome::Delivered);
    assert_eq!(received.unwrap().len(), 3);

    let mut staging = OpfsStaging::open(&root().await, &manifest, &[], &sender_key).await;
    assert_intact(&manifest, staging.as_mut().unwrap());
    drop(staging); // removing needs every file closed
    remove_staged(&root().await, &manifest, &[], &sender_key)
        .await
        .unwrap();
}

#[wasm_bindgen_test]
async fn a_phone_sends_files_into_the_browser() {
    phone_to_browser(None).await;
}

/// With one piece in flight, the sender's first piece (the packed small files, a few KiB)
/// fills its window and it waits for an `ACK`. The receiver is far below the bytes it acks by
/// and has only just started, so only its ack timer sends that `ACK`: this hangs unless the
/// browser's timers work.
#[wasm_bindgen_test]
async fn acks_keep_flowing_on_the_browsers_timer() {
    phone_to_browser(Some(1)).await;
}
