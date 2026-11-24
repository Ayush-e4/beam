use super::*;
use crate::ErrorCategory;

const TOKEN: &[u8; 16] = b"one-time-token!!";

fn keypair() -> StaticKeypair {
    StaticKeypair::generate().unwrap()
}

/// Drives both sides to completion, moving every message across.
fn run(mut initiator: Handshake, mut responder: Handshake) -> Result<(Established, Established)> {
    while !(initiator.is_finished() && responder.is_finished()) {
        let (from, to) = if initiator.is_my_turn() {
            (&mut initiator, &mut responder)
        } else {
            (&mut responder, &mut initiator)
        };
        let message = from.write_message()?;
        to.read_message(&message)?;
    }
    Ok((initiator.finish()?, responder.finish()?))
}

fn pairing(
    cipher: Cipher,
    scanner: &StaticKeypair,
    phone: &StaticKeypair,
) -> Result<(Established, Established)> {
    run(
        Handshake::pairing_scanner(scanner, cipher, Role::Initiator, TOKEN, phone.public())?,
        Handshake::pairing_displayer(phone, cipher, Role::Responder, TOKEN)?,
    )
}

/// Confirms both sides derived matching keys by sending one message each way.
fn assert_keys_match(a: Established, b: Established) {
    let (mut a, mut b) = (a.into_transport(), b.into_transport());
    let mut sealed = [0u8; 64];
    let mut opened = [0u8; 64];
    let len = a.write_message(b"ping", &mut sealed).unwrap();
    let n = b.read_message(&sealed[..len], &mut opened).unwrap();
    assert_eq!(&opened[..n], b"ping");
    let len = b.write_message(b"pong", &mut sealed).unwrap();
    let n = a.read_message(&sealed[..len], &mut opened).unwrap();
    assert_eq!(&opened[..n], b"pong");
}

#[test]
fn pairing_authenticates_both_sides_with_either_cipher() {
    for cipher in [Cipher::AesGcm, Cipher::ChaChaPoly] {
        let (scanner, phone) = (keypair(), keypair());
        let (s, p) = pairing(cipher, &scanner, &phone).unwrap();
        assert_eq!(s.peer(), phone.public());
        assert_eq!(p.peer(), scanner.public());
        assert_eq!(s.verification_code(), p.verification_code());
        assert_keys_match(s, p);
    }
}

#[test]
fn pairing_takes_three_messages() {
    let (scanner, phone) = (keypair(), keypair());
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
    assert!(!i.is_finished());
    r.read_message(&i.write_message().unwrap()).unwrap();
    assert!(i.is_finished() && r.is_finished());
}

#[test]
fn every_session_gets_a_fresh_verification_code() {
    let (scanner, phone) = (keypair(), keypair());
    let first = pairing(Cipher::AesGcm, &scanner, &phone).unwrap().0;
    let second = pairing(Cipher::AesGcm, &scanner, &phone).unwrap().0;
    assert_ne!(first.verification_code(), second.verification_code());
}

#[test]
fn wrong_pairing_token_fails() {
    let (scanner, phone) = (keypair(), keypair());
    let err = run(
        Handshake::pairing_scanner(
            &scanner,
            Cipher::AesGcm,
            Role::Initiator,
            b"stolen-old-token",
            phone.public(),
        )
        .unwrap(),
        Handshake::pairing_displayer(&phone, Cipher::AesGcm, Role::Responder, TOKEN).unwrap(),
    )
    .err()
    .unwrap();
    assert_eq!(err.category(), ErrorCategory::Crypto);
}

#[test]
fn impostor_phone_is_rejected_before_the_scanner_reveals_itself() {
    // The QR names the real phone, but an attacker answers the connection.
    let (scanner, phone, attacker) = (keypair(), keypair(), keypair());
    let mut i = Handshake::pairing_scanner(
        &scanner,
        Cipher::AesGcm,
        Role::Initiator,
        TOKEN,
        phone.public(),
    )
    .unwrap();
    let mut r =
        Handshake::pairing_displayer(&attacker, Cipher::AesGcm, Role::Responder, TOKEN).unwrap();
    r.read_message(&i.write_message().unwrap()).unwrap();
    let err = i.read_message(&r.write_message().unwrap()).unwrap_err();
    assert!(matches!(
        err,
        BeamError::UnexpectedPeerKey | BeamError::Handshake(_)
    ));
    assert!(i.finish().is_err());
}

#[test]
fn known_device_handshake_takes_one_round_trip() {
    let (laptop, phone) = (keypair(), keypair());
    let mut i = Handshake::known_initiator(&laptop, Cipher::ChaChaPoly, phone.public()).unwrap();
    let mut r = Handshake::known_responder(&phone, Cipher::ChaChaPoly).unwrap();
    r.read_message(&i.write_message().unwrap()).unwrap();
    i.read_message(&r.write_message().unwrap()).unwrap();
    let (a, b) = (i.finish().unwrap(), r.finish().unwrap());
    assert_eq!(a.peer(), phone.public());
    assert_eq!(b.peer(), laptop.public());
    assert_keys_match(a, b);
}

#[test]
fn known_device_handshake_fails_against_a_different_device() {
    let (laptop, phone, other) = (keypair(), keypair(), keypair());
    let result = run(
        Handshake::known_initiator(&laptop, Cipher::AesGcm, phone.public()).unwrap(),
        Handshake::known_responder(&other, Cipher::AesGcm).unwrap(),
    );
    assert!(result.is_err());
}

#[test]
fn mismatched_ciphers_fail() {
    let (scanner, phone) = (keypair(), keypair());
    let result = run(
        Handshake::pairing_scanner(
            &scanner,
            Cipher::AesGcm,
            Role::Initiator,
            TOKEN,
            phone.public(),
        )
        .unwrap(),
        Handshake::pairing_displayer(&phone, Cipher::ChaChaPoly, Role::Responder, TOKEN).unwrap(),
    );
    assert!(result.is_err());
}

#[test]
fn tampered_message_is_rejected() {
    let (scanner, phone) = (keypair(), keypair());
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
    let mut reply = r.write_message().unwrap();
    let last = reply.len() - 1;
    reply[last] ^= 0x01;
    assert!(matches!(
        i.read_message(&reply),
        Err(BeamError::Handshake(_))
    ));
}

#[test]
fn misuse_is_reported_not_panicked() {
    let phone = keypair();
    let mut r =
        Handshake::pairing_displayer(&phone, Cipher::AesGcm, Role::Responder, TOKEN).unwrap();
    assert!(!r.is_my_turn());
    assert!(r.write_message().is_err());
    assert!(r.read_message(&[0u8; 3]).is_err());
    assert!(matches!(r.finish(), Err(BeamError::Handshake(_))));
}

#[test]
fn pairing_token_length_is_bounded() {
    let phone = keypair();
    let short = [0u8; MIN_PAIRING_TOKEN_LEN - 1];
    let long = [0u8; MAX_PAIRING_TOKEN_LEN + 1];
    assert!(Handshake::pairing_displayer(&phone, Cipher::AesGcm, Role::Responder, &short).is_err());
    assert!(Handshake::pairing_displayer(&phone, Cipher::AesGcm, Role::Responder, &long).is_err());
    let max = [0u8; MAX_PAIRING_TOKEN_LEN];
    assert!(Handshake::pairing_displayer(&phone, Cipher::AesGcm, Role::Responder, &max).is_ok());
}

#[test]
fn browser_shows_the_code_and_connects_out_to_the_phone() {
    // A browser cannot accept connections, so it displays the QR and initiates; the phone
    // scans and responds.
    let (browser, phone) = (keypair(), keypair());
    let (b, p) = run(
        Handshake::pairing_displayer(&browser, Cipher::AesGcm, Role::Initiator, TOKEN).unwrap(),
        Handshake::pairing_scanner(
            &phone,
            Cipher::AesGcm,
            Role::Responder,
            TOKEN,
            browser.public(),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(b.peer(), phone.public());
    assert_eq!(p.peer(), browser.public());
    assert_eq!(b.verification_code(), p.verification_code());
}

#[test]
fn phone_scanning_rejects_a_connector_that_is_not_the_displayed_browser() {
    // Someone who learned the token connects first, but their key is not the one in the QR.
    let (browser, phone, attacker) = (keypair(), keypair(), keypair());
    let result = run(
        Handshake::pairing_displayer(&attacker, Cipher::AesGcm, Role::Initiator, TOKEN).unwrap(),
        Handshake::pairing_scanner(
            &phone,
            Cipher::AesGcm,
            Role::Responder,
            TOKEN,
            browser.public(),
        )
        .unwrap(),
    );
    assert!(result.is_err());
}
