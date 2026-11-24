use proptest::prelude::*;

use super::*;
use crate::frame::{FrameType, MAX_PAYLOAD_LEN};
use crate::handshake::{Cipher, Handshake, Role};
use crate::keys::StaticKeypair;

const TOKEN: &[u8; 16] = b"one-time-token!!";

/// Pairs two fresh devices and returns (scanner session, phone session).
fn pair() -> (Session, Session) {
    let (scanner, phone) = (
        StaticKeypair::generate().unwrap(),
        StaticKeypair::generate().unwrap(),
    );
    let mut i = Handshake::pairing_scanner(
        &scanner,
        Cipher::AesGcm,
        Role::Initiator,
        TOKEN,
        phone.public(),
    )
    .unwrap();
    let mut r =
        Handshake::pairing_displayer(&phone, Cipher::AesGcm, Role::Responder, TOKEN).unwrap();
    r.read_message(&i.write_message().unwrap()).unwrap();
    i.read_message(&r.write_message().unwrap()).unwrap();
    r.read_message(&i.write_message().unwrap()).unwrap();
    (
        Session::new(i.finish().unwrap()),
        Session::new(r.finish().unwrap()),
    )
}

fn frame(frame_type: FrameType, payload: &[u8]) -> Frame {
    Frame::new(frame_type, 7, payload.to_vec()).unwrap()
}

fn sealed(session: &mut Session, frame: &Frame) -> Vec<u8> {
    let mut wire = Vec::new();
    session.seal(frame, &mut wire).unwrap();
    wire
}

/// Splits wire bytes into whole records.
fn records(wire: &[u8]) -> Vec<&[u8]> {
    let mut out = Vec::new();
    let mut rest = wire;
    while !rest.is_empty() {
        let len = LEN_PREFIX + usize::from(u16::from_be_bytes([rest[0], rest[1]]));
        out.push(&rest[..len]);
        rest = &rest[len..];
    }
    out
}

#[test]
fn frames_flow_both_ways_and_peers_are_known() {
    let (mut a, mut b) = pair();
    assert_ne!(a.peer(), b.peer());
    let request = frame(FrameType::Ping, b"hi");
    assert_eq!(b.receive(&sealed(&mut a, &request)).unwrap(), vec![request]);
    let reply = frame(FrameType::Pong, b"hello back");
    assert_eq!(a.receive(&sealed(&mut b, &reply)).unwrap(), vec![reply]);
}

#[test]
fn ciphertext_hides_the_plaintext() {
    let (mut a, _) = pair();
    let secret = b"a very recognisable secret payload";
    let wire = sealed(&mut a, &frame(FrameType::Clipboard, secret));
    assert!(!wire.windows(secret.len()).any(|w| w == secret));
}

#[test]
fn largest_frame_spans_many_records() {
    let (mut a, mut b) = pair();
    let payload: Vec<u8> = (0..=250u8).cycle().take(MAX_PAYLOAD_LEN).collect();
    let big = Frame::new(FrameType::Chunk, 1, payload).unwrap();
    let wire = sealed(&mut a, &big);
    assert!(records(&wire).len() > MAX_PAYLOAD_LEN / MAX_RECORD_PLAINTEXT);
    assert!(
        records(&wire)
            .iter()
            .all(|r| r.len() <= LEN_PREFIX + MAX_RECORD_LEN)
    );
    assert_eq!(b.receive(&wire).unwrap(), vec![big]);
}

#[test]
fn byte_at_a_time_delivery_reassembles_frames() {
    let (mut a, mut b) = pair();
    let frames = [
        frame(FrameType::Hello, b"v1"),
        frame(FrameType::Ack, &[9; 300]),
    ];
    let mut wire = Vec::new();
    for f in &frames {
        a.seal(f, &mut wire).unwrap();
    }
    let mut got = Vec::new();
    for byte in wire {
        got.extend(b.receive(&[byte]).unwrap());
    }
    assert_eq!(got, frames);
}

#[test]
fn keys_rotate_without_interrupting_the_stream() {
    let (a, b) = pair();
    let (mut a, mut b) = (a.with_rekey_interval(3), b.with_rekey_interval(3));
    for n in 0..20u8 {
        let f = frame(FrameType::Ping, &[n]);
        assert_eq!(b.receive(&sealed(&mut a, &f)).unwrap(), vec![f]);
    }
}

#[test]
fn tampered_record_is_rejected_and_breaks_the_session() {
    let (mut a, mut b) = pair();
    let mut wire = sealed(&mut a, &frame(FrameType::Ping, b"x"));
    let last = wire.len() - 1;
    wire[last] ^= 0x80;
    assert!(matches!(b.receive(&wire), Err(BeamError::RecordRejected)));
    let good = sealed(&mut a, &frame(FrameType::Ping, b"y"));
    assert!(matches!(b.receive(&good), Err(BeamError::SessionBroken)));
}

#[test]
fn replayed_record_is_rejected() {
    let (mut a, mut b) = pair();
    let wire = sealed(&mut a, &frame(FrameType::Ping, b"once"));
    b.receive(&wire).unwrap();
    assert!(matches!(b.receive(&wire), Err(BeamError::RecordRejected)));
}

#[test]
fn reordered_records_are_rejected() {
    let (mut a, mut b) = pair();
    let _first = sealed(&mut a, &frame(FrameType::Ping, b"1"));
    let second = sealed(&mut a, &frame(FrameType::Ping, b"2"));
    // Delivering `second` first is the same as losing `first`: the nonce does not match.
    assert!(matches!(b.receive(&second), Err(BeamError::RecordRejected)));
}

#[test]
fn records_from_another_session_are_rejected() {
    let (mut a, _) = pair();
    let (_, mut other) = pair();
    let wire = sealed(&mut a, &frame(FrameType::Ping, b"x"));
    assert!(matches!(
        other.receive(&wire),
        Err(BeamError::RecordRejected)
    ));
}

#[test]
fn records_too_short_to_hold_a_tag_are_rejected() {
    let (_, mut b) = pair();
    assert!(matches!(b.receive(&[0, 0]), Err(BeamError::RecordRejected)));
    let (_, mut b) = pair();
    let mut short = vec![0, 16];
    short.extend([0u8; 16]);
    assert!(matches!(b.receive(&short), Err(BeamError::RecordRejected)));
}

#[test]
fn errors_are_crypto_category() {
    assert_eq!(
        BeamError::RecordRejected.category(),
        crate::ErrorCategory::Crypto
    );
    assert_eq!(
        BeamError::SessionBroken.category(),
        crate::ErrorCategory::Crypto
    );
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(32))]

    #[test]
    fn any_split_of_the_stream_yields_the_same_frames(
        payloads in prop::collection::vec(prop::collection::vec(any::<u8>(), 0..200_000), 1..5),
        cuts in prop::collection::vec(1usize..100_000, 0..20),
    ) {
        let (mut a, mut b) = pair();
        let frames: Vec<Frame> = payloads
            .into_iter()
            .map(|p| Frame::new(FrameType::Chunk, 3, p).unwrap())
            .collect();
        let mut wire = Vec::new();
        for f in &frames {
            a.seal(f, &mut wire).unwrap();
        }
        let mut got = Vec::new();
        let mut rest = wire.as_slice();
        for cut in cuts {
            let (head, tail) = rest.split_at(cut.min(rest.len()));
            got.extend(b.receive(head).unwrap());
            rest = tail;
        }
        got.extend(b.receive(rest).unwrap());
        prop_assert_eq!(got, frames);
    }
}
