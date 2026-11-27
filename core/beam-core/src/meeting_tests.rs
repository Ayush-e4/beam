use super::*;
use crate::{BeamError, Cipher, PairingInvite, SignalFrom, SignalMessage};

fn offer() -> SignalMessage {
    SignalMessage::Offer {
        hints: vec!["192.168.1.20:47123".parse().unwrap()],
        certificate_hash: [9; 32],
    }
}

#[test]
fn both_devices_find_the_same_room_and_seal() {
    let phone = StaticKeypair::generate().unwrap();
    let browser = StaticKeypair::generate().unwrap();
    let at_phone = Meeting::between(&phone, &browser.public()).unwrap();
    let at_browser = Meeting::between(&browser, &phone.public()).unwrap();
    assert_eq!(at_phone.room(), at_browser.room());

    let sealed = at_phone.seal().seal(SignalFrom::Scanner, &offer()).unwrap();
    let opened = at_browser
        .seal()
        .open(SignalFrom::Scanner, &sealed)
        .unwrap();
    assert_eq!(opened, offer());
    assert!(
        at_browser
            .seal()
            .open(SignalFrom::Displayer, &sealed)
            .is_err(),
        "a phone's offer can't be bounced back as the browser's"
    );
}

#[test]
fn every_pair_meets_somewhere_else() {
    let phone = StaticKeypair::generate().unwrap();
    let browser = StaticKeypair::generate().unwrap();
    let other = StaticKeypair::generate().unwrap();
    let ours = Meeting::between(&phone, &browser.public()).unwrap();
    let theirs = Meeting::between(&phone, &other.public()).unwrap();
    assert_ne!(ours.room(), theirs.room());

    let sealed = ours.seal().seal(SignalFrom::Scanner, &offer()).unwrap();
    let outsider = Meeting::between(&other, &phone.public()).unwrap();
    assert!(matches!(
        outsider.seal().open(SignalFrom::Scanner, &sealed),
        Err(BeamError::Handshake(_))
    ));
}

#[test]
fn meeting_rooms_are_not_pairing_rooms() {
    let phone = StaticKeypair::generate().unwrap();
    let browser = StaticKeypair::generate().unwrap();
    let invite = PairingInvite::new(browser.public(), Cipher::ChaChaPoly, Vec::new()).unwrap();
    let meeting = Meeting::between(&phone, &browser.public()).unwrap();
    assert_ne!(meeting.room(), invite.room());

    let sealed = meeting.seal().seal(SignalFrom::Scanner, &offer()).unwrap();
    let pairing = SignalSeal::for_invite(&invite).unwrap();
    assert!(pairing.open(SignalFrom::Scanner, &sealed).is_err());
}

#[test]
fn a_weak_key_has_no_meeting() {
    let phone = StaticKeypair::generate().unwrap();
    let weak = PublicKey::from_bytes([0; 32]);
    assert!(matches!(
        Meeting::between(&phone, &weak),
        Err(BeamError::Handshake(_))
    ));
}
