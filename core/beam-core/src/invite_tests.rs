use super::*;

fn invite(hints: Vec<SocketAddr>) -> PairingInvite {
    PairingInvite::new(PublicKey::from_bytes([7; 32]), Cipher::AesGcm, hints).unwrap()
}

fn lan() -> Vec<SocketAddr> {
    vec![
        "192.168.1.23:41641".parse().unwrap(),
        "[fe80::1c2b:3aff:fe4d:5e6f]:41641".parse().unwrap(),
    ]
}

#[test]
fn round_trips_through_text() {
    for hints in [Vec::new(), lan()] {
        let original = invite(hints);
        let text = original.encode();
        assert!(text.starts_with(INVITE_PREFIX));
        assert_eq!(PairingInvite::decode(&text).unwrap(), original);
    }
}

#[test]
fn fits_the_qr_alphanumeric_set_and_stays_small() {
    let text = invite(lan()).encode();
    let qr_alphanumeric =
        |c: char| c.is_ascii_digit() || c.is_ascii_uppercase() || " $%*+-./:".contains(c);
    assert!(text.chars().all(qr_alphanumeric), "{text}");
    // Two hints, one IPv6: 1+1+16+32+1 + 7 + 19 = 77 bytes → 124 base32 chars + prefix.
    assert_eq!(text.len(), INVITE_PREFIX.len() + 124);
}

#[test]
fn each_invite_has_its_own_token_and_room() {
    let (a, b) = (invite(Vec::new()), invite(Vec::new()));
    assert_ne!(a.token, b.token);
    assert_ne!(a.room(), b.room());
    assert_eq!(
        a.room(),
        a.clone().room(),
        "the room follows from the token"
    );
}

#[test]
fn surrounding_whitespace_is_ignored() {
    let original = invite(lan());
    let text = format!("  {}\n", original.encode());
    assert_eq!(PairingInvite::decode(&text).unwrap(), original);
}

#[test]
fn other_codes_are_rejected_with_a_reason() {
    let good = invite(lan()).encode();
    let body = &good[INVITE_PREFIX.len()..];
    let cases = [
        ("https://example.com".to_owned(), "not a Beam pairing code"),
        (format!("{INVITE_PREFIX}{}", body.to_lowercase()), "damaged"),
        (format!("{INVITE_PREFIX}{}", &body[..40]), "truncated"),
        (encode_raw(&[2]), "newer version"),
        (encode_raw(&[1, 9]), "unknown cipher"),
    ];
    for (text, why) in cases {
        let error = PairingInvite::decode(&text).unwrap_err().to_string();
        assert!(error.contains(why), "{text}: {error}");
    }
}

#[test]
fn hints_are_checked() {
    let five: Vec<SocketAddr> = (1..=5)
        .map(|n| SocketAddr::from(([10, 0, 0, n], 9)))
        .collect();
    assert!(PairingInvite::new(PublicKey::from_bytes([1; 32]), Cipher::AesGcm, five).is_err());
    let no_port = vec![SocketAddr::from(([10, 0, 0, 1], 0))];
    assert!(PairingInvite::new(PublicKey::from_bytes([1; 32]), Cipher::AesGcm, no_port).is_err());

    let mut bytes = raw_header();
    bytes.push(5); // more hints than allowed
    assert!(PairingInvite::decode(&encode_raw(&bytes)).is_err());
    let mut bytes = raw_header();
    bytes.extend_from_slice(&[1, 5, 10, 0, 0, 1, 0, 9]); // unknown family 5
    assert!(PairingInvite::decode(&encode_raw(&bytes)).is_err());
    let mut bytes = raw_header();
    bytes.extend_from_slice(&[0, 0xFF]); // no hints, then junk
    let error = PairingInvite::decode(&encode_raw(&bytes))
        .unwrap_err()
        .to_string();
    assert!(error.contains("trailing"), "{error}");
}

fn raw_header() -> Vec<u8> {
    let mut bytes = vec![1, 0];
    bytes.extend_from_slice(&[3; 16]);
    bytes.extend_from_slice(&[7; 32]);
    bytes
}

fn encode_raw(bytes: &[u8]) -> String {
    format!("{INVITE_PREFIX}{}", base32::encode(bytes))
}
