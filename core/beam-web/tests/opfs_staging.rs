//! Receiving into OPFS in a real browser worker: the sender reads files from memory and the
//! receiver stages them through [`OpfsStaging`], exactly as the web app's worker will.
//!
//! Run with `tools/web-staging-test.sh`.
#![cfg(target_family = "wasm")]
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use beam_core::messages::{Accept, Resume};
use beam_core::{
    Manifest, PreparedTransfer, Receiver, Sender, TransferStaging, prepare_readable,
    prepare_readable_with_id,
};
use beam_web::opfs_staging::{OpfsStaging, remove_staged};
use common::{assert_intact, readable, root};
use wasm_bindgen_test::{wasm_bindgen_test, wasm_bindgen_test_configure};

wasm_bindgen_test_configure!(run_in_dedicated_worker);

const SENDER: &[u8] = &[7; 32];

fn prepare() -> PreparedTransfer {
    prepare_readable(readable()).unwrap()
}

/// The same files again, as a sender restarted for a resume prepares them.
fn prepare_again(manifest: &Manifest) -> PreparedTransfer {
    prepare_readable_with_id(readable(), manifest.transfer_id()).unwrap()
}

async fn staging(manifest: &Manifest) -> Box<OpfsStaging> {
    Box::new(
        OpfsStaging::open(&root().await, manifest, &[], SENDER)
            .await
            .unwrap(),
    )
}

/// Sends until the sender has nothing left, acknowledging every checkpoint.
fn pump(sender: &mut Sender, receiver: &mut Receiver) {
    loop {
        while sender.can_send() {
            let Some(payload) = sender.next_chunk().unwrap() else {
                break;
            };
            receiver.handle_chunk(&payload).unwrap();
        }
        let acks = receiver.checkpoint().unwrap();
        if acks.is_empty() {
            break;
        }
        sender.handle_ack(&acks);
    }
}

/// Every file's bytes, read back from a fresh staging over the same OPFS files.
async fn assert_staged_intact(manifest: &Manifest) {
    assert_intact(manifest, &mut *staging(manifest).await);
}

#[wasm_bindgen_test]
async fn a_transfer_lands_in_opfs_intact() {
    let prepared = prepare();
    let manifest = prepared.manifest().clone();
    let staged = staging(&manifest).await;
    let mut receiver = Receiver::open_staged(manifest.clone(), &[], staged).unwrap();
    let mut sender = Sender::new(prepared);
    let transfer_id = manifest.transfer_id();
    sender
        .accept(&Accept {
            transfer_id,
            items: Vec::new(),
        })
        .unwrap();
    pump(&mut sender, &mut receiver);
    assert!(sender.is_complete());
    let paths = receiver.finish().unwrap();
    assert_eq!(paths.len(), 3);

    assert_staged_intact(&manifest).await;
    remove_staged(&root().await, &manifest, &[], SENDER)
        .await
        .unwrap();
    let mut fresh = staging(&manifest).await;
    assert_eq!(
        fresh.journal().unwrap().size().unwrap(),
        0,
        "staging was removed"
    );
}

#[wasm_bindgen_test]
async fn a_transfer_resumes_after_the_worker_stops() {
    let prepared = prepare();
    let manifest = prepared.manifest().clone();
    let transfer_id = manifest.transfer_id();
    {
        let staged = staging(&manifest).await;
        let mut receiver = Receiver::open_staged(manifest.clone(), &[], staged).unwrap();
        let mut sender = Sender::new(prepared);
        sender
            .accept(&Accept {
                transfer_id,
                items: Vec::new(),
            })
            .unwrap();
        for _ in 0..2 {
            let payload = sender.next_chunk().unwrap().unwrap();
            receiver.handle_chunk(&payload).unwrap();
        }
        receiver.checkpoint().unwrap();
        let unacknowledged = sender.next_chunk().unwrap().unwrap();
        receiver.handle_chunk(&unacknowledged).unwrap();
    } // the tab closes: every OPFS handle is let go

    let mut receiver =
        Receiver::open_staged(manifest.clone(), &[], staging(&manifest).await).unwrap();
    assert!(receiver.resumed());
    let have = receiver.have();
    assert_ne!(have, Vec::new(), "the first session left chunks behind");
    let mut sender = Sender::new(prepare_again(&manifest));
    sender
        .resume(&Resume {
            transfer_id,
            items: Vec::new(),
            have,
        })
        .unwrap();
    pump(&mut sender, &mut receiver);
    receiver.finish().unwrap();

    assert_staged_intact(&manifest).await;
    remove_staged(&root().await, &manifest, &[], SENDER)
        .await
        .unwrap();
}

#[wasm_bindgen_test]
async fn a_second_tab_cannot_open_a_transfer_in_progress() {
    let manifest = prepare().manifest().clone();
    let first = staging(&manifest).await;
    let second = OpfsStaging::open(&root().await, &manifest, &[], SENDER).await;
    assert!(second.is_err(), "the first tab holds it");
    drop(first);
    OpfsStaging::open(&root().await, &manifest, &[], SENDER)
        .await
        .expect("free once the first tab lets go");
    remove_staged(&root().await, &manifest, &[], SENDER)
        .await
        .unwrap();
}
