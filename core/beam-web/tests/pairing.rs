//! Opening the phone's sealed offer in a real browser, as the web app's worker does after
//! reading it from the signaling room. The phone's side is beam-core, sealing with the token
//! it scanned from the invite's text.
//!
//! Run with `tools/web-engine-test.sh`.
#![cfg(target_family = "wasm")]
#![allow(clippy::unwrap_used)]

use std::net::SocketAddr;

use beam_core::{PairingInvite, SignalFrom, SignalMessage, SignalSeal};
use beam_web::{Identity, Invite};
use js_sys::Reflect;
use wasm_bindgen::JsValue;
use wasm_bindgen_test::{wasm_bindgen_test, wasm_bindgen_test_configure};

wasm_bindgen_test_configure!(run_in_dedicated_worker);

const HASH: [u8; 32] = [0xab; 32];

fn invite() -> Invite {
    Invite::new(&Identity::generate().unwrap(), "chacha-poly").unwrap()
}

/// What the phone that scanned `invite` posts, sealed as `from`.
fn sealed(invite: &Invite, from: SignalFrom, hints: &[&str]) -> Vec<u8> {
    let scanned = PairingInvite::decode(&invite.text()).unwrap();
    let offer = SignalMessage::Offer {
        hints: hints
            .iter()
            .map(|hint| hint.parse::<SocketAddr>().unwrap())
            .collect(),
        certificate_hash: HASH,
    };
    SignalSeal::for_invite(&scanned)
        .unwrap()
        .seal(from, &offer)
        .unwrap()
}

fn category(error: &JsValue) -> String {
    Reflect::get(error, &"category".into())
        .unwrap()
        .as_string()
        .unwrap()
}

#[wasm_bindgen_test]
fn the_offer_says_where_to_connect_and_what_to_pin() {
    let invite = invite();
    let hints = ["192.168.1.20:47123", "[2001:db8::7]:47123"];
    let offer = invite
        .open_offer(&sealed(&invite, SignalFrom::Scanner, &hints))
        .unwrap();
    assert_eq!(
        offer.addresses(),
        hints,
        "ready for https://{{address}}/beam"
    );
    assert_eq!(offer.certificate_hash(), HASH);
}

#[wasm_bindgen_test]
fn another_pairings_offer_is_refused() {
    let theirs = invite();
    let sealed = sealed(&theirs, SignalFrom::Scanner, &["192.168.1.20:47123"]);
    let error = invite().open_offer(&sealed).err().unwrap();
    assert_eq!(category(&error), "crypto");
}

#[wasm_bindgen_test]
fn the_browsers_own_message_bounced_back_is_refused() {
    let invite = invite();
    let echoed = sealed(&invite, SignalFrom::Displayer, &["192.168.1.20:47123"]);
    let error = invite.open_offer(&echoed).err().unwrap();
    assert_eq!(category(&error), "crypto");
}

#[wasm_bindgen_test]
fn an_offer_without_an_address_is_refused() {
    let invite = invite();
    let error = invite
        .open_offer(&sealed(&invite, SignalFrom::Scanner, &[]))
        .err()
        .unwrap();
    assert_eq!(category(&error), "invalid");
}

#[wasm_bindgen_test]
fn the_discovery_key_is_stable_for_an_identity() {
    let identity = Identity::generate().unwrap();
    let again = Identity::from_private_key(&identity.private_key()).unwrap();
    assert_eq!(identity.discovery_key().len(), 32);
    assert_eq!(identity.discovery_key(), again.discovery_key());
}
