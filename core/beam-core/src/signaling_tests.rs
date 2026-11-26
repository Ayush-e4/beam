// Test code reports a broken setup by panicking.
#![allow(clippy::unwrap_used)]

use std::net::SocketAddr;

use super::*;
use crate::{Cipher, ErrorCategory, StaticKeypair};

fn invite() -> PairingInvite {
    let displayer = StaticKeypair::generate().unwrap().public();
    PairingInvite::new(displayer, Cipher::AesGcm, Vec::new()).unwrap()
}

fn offer() -> SignalMessage {
    let hints: Vec<SocketAddr> = vec![
        "192.168.251.204:47124".parse().unwrap(),
        "[fe80::1]:47124".parse().unwrap(),
    ];
    SignalMessage::Offer {
        hints,
        certificate_hash: [7; 32],
    }
}

#[test]
fn both_sides_of_a_pairing_open_what_the_other_sealed() {
    let invite = invite();
    let phone = SignalSeal::for_invite(&invite).unwrap();
    let browser =
        SignalSeal::for_invite(&PairingInvite::decode(&invite.encode()).unwrap()).unwrap();

    let sealed = phone.seal(SignalFrom::Scanner, &offer()).unwrap();

    assert_eq!(browser.open(SignalFrom::Scanner, &sealed).unwrap(), offer());
}

#[test]
fn sealing_twice_looks_different() {
    let seal = SignalSeal::for_invite(&invite()).unwrap();
    let first = seal.seal(SignalFrom::Scanner, &offer()).unwrap();
    let second = seal.seal(SignalFrom::Scanner, &offer()).unwrap();
    assert_ne!(first, second);
}

#[test]
fn another_pairing_cannot_open_it() {
    let sealed = SignalSeal::for_invite(&invite())
        .unwrap()
        .seal(SignalFrom::Scanner, &offer())
        .unwrap();
    let stranger = SignalSeal::for_invite(&invite()).unwrap();
    let error = stranger.open(SignalFrom::Scanner, &sealed).unwrap_err();
    assert_eq!(error.category(), ErrorCategory::Crypto);
}

#[test]
fn a_message_does_not_open_as_coming_from_the_other_side() {
    let seal = SignalSeal::for_invite(&invite()).unwrap();
    let sealed = seal.seal(SignalFrom::Scanner, &offer()).unwrap();
    assert!(seal.open(SignalFrom::Displayer, &sealed).is_err());
}

#[test]
fn any_change_on_the_way_is_caught() {
    let seal = SignalSeal::for_invite(&invite()).unwrap();
    let sealed = seal.seal(SignalFrom::Scanner, &offer()).unwrap();
    for i in 1..sealed.len() {
        let mut changed = sealed.clone();
        changed[i] ^= 1;
        assert!(
            seal.open(SignalFrom::Scanner, &changed).is_err(),
            "byte {i}"
        );
    }
    assert!(
        seal.open(SignalFrom::Scanner, &sealed[..sealed.len() - 1])
            .is_err()
    );
}

#[test]
fn newer_versions_and_scraps_are_refused() {
    let seal = SignalSeal::for_invite(&invite()).unwrap();
    let mut sealed = seal.seal(SignalFrom::Scanner, &offer()).unwrap();
    sealed[0] = 2;
    assert_eq!(
        seal.open(SignalFrom::Scanner, &sealed)
            .unwrap_err()
            .category(),
        ErrorCategory::Protocol
    );
    for scrap in [&[][..], &[1], &[1, 0, 0]] {
        assert!(seal.open(SignalFrom::Scanner, scrap).is_err());
    }
}

#[test]
fn plain_messages_round_trip_and_damage_is_refused() {
    let bytes = encode(&offer()).unwrap();
    assert_eq!(decode(&bytes).unwrap(), offer());
    assert!(decode(&bytes[..bytes.len() - 1]).is_err());
    assert!(decode(&[bytes.as_slice(), &[0]].concat()).is_err());
    assert!(decode(&[9]).is_err());
    assert!(decode(&[]).is_err());
}

#[test]
fn an_offer_with_a_bad_hint_is_not_sealed() {
    let seal = SignalSeal::for_invite(&invite()).unwrap();
    let no_port = SignalMessage::Offer {
        hints: vec!["10.0.0.1:0".parse().unwrap()],
        certificate_hash: [0; 32],
    };
    assert!(seal.seal(SignalFrom::Scanner, &no_port).is_err());
}
