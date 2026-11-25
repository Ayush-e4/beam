use super::*;

fn identity() -> StaticKeypair {
    StaticKeypair::generate().unwrap()
}

#[test]
fn a_beacon_is_recognised_with_the_right_key_only() {
    let key = DiscoveryKey::derive(&identity(), 0);
    let other = DiscoveryKey::derive(&identity(), 0);
    let beacon = Beacon::new(&key).unwrap();
    assert!(beacon.matches(&key));
    assert!(!beacon.matches(&other));
    assert_eq!(beacon.find([&other, &key]), Some(1));
    assert_eq!(beacon.find([&other]), None);
}

#[test]
fn keys_are_stable_per_generation() {
    let me = identity();
    assert_eq!(DiscoveryKey::derive(&me, 3), DiscoveryKey::derive(&me, 3));
    let old = DiscoveryKey::derive(&me, 0);
    let new = DiscoveryKey::derive(&me, 1);
    assert_ne!(old, new);
    assert!(!Beacon::new(&new).unwrap().matches(&old));
}

#[test]
fn beacons_change_every_time() {
    let key = DiscoveryKey::derive(&identity(), 0);
    let a = Beacon::new(&key).unwrap();
    let b = Beacon::new(&key).unwrap();
    assert_ne!(a.encode(), b.encode());
    assert!(a.matches(&key) && b.matches(&key));
}

#[test]
fn beacons_round_trip_as_text_in_any_case() {
    let key = DiscoveryKey::derive(&identity(), 0);
    let beacon = Beacon::new(&key).unwrap();
    let text = beacon.encode();
    assert_eq!(text.len(), 26);
    assert_eq!(Beacon::parse(&text).unwrap(), beacon);
    assert_eq!(Beacon::parse(&text.to_ascii_lowercase()).unwrap(), beacon);
}

#[test]
fn a_tampered_beacon_is_not_recognised() {
    let key = DiscoveryKey::derive(&identity(), 0);
    let beacon = Beacon::new(&key).unwrap();
    let mut forged = beacon;
    forged.tag[0] ^= 1;
    assert!(!forged.matches(&key));
    let mut moved = beacon;
    moved.nonce[7] ^= 1;
    assert!(!moved.matches(&key));
}

#[test]
fn the_tag_is_fixed_for_a_known_key_and_nonce() {
    let key = DiscoveryKey::from_slice(&[7; DISCOVERY_KEY_LEN]).unwrap();
    let beacon = Beacon::with_nonce(&key, [1, 2, 3, 4, 5, 6, 7, 8]);
    let mut hasher = blake3::Hasher::new_keyed(&[7; DISCOVERY_KEY_LEN]);
    hasher
        .update(b"beam beacon")
        .update(&[1, 2, 3, 4, 5, 6, 7, 8]);
    assert_eq!(beacon.tag, hasher.finalize().as_bytes()[..TAG_LEN]);
}

#[test]
fn other_text_is_rejected() {
    for text in [
        "",
        "BEAM",
        "AAAAAAAAAAAAAAAAAAAAAAAAA",
        "AAAAAAAAAAAAAAAAAAAAAAAAAAAA",
        "!",
    ] {
        assert!(Beacon::parse(text).is_err(), "{text}");
    }
    assert!(DiscoveryKey::from_slice(&[0; 31]).is_err());
}
