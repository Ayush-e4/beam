//! Beacons heard on the network never panic, and any beacon accepted re-encodes to the same
//! text (ignoring case) and can be checked against a key.
#![no_main]

use beam_core::{Beacon, DiscoveryKey};
use beam_core_fuzz::base32;
use libfuzzer_sys::fuzz_target;

fn check(text: &str) {
    if let Ok(beacon) = Beacon::parse(text) {
        assert_eq!(beacon.encode(), text.to_ascii_uppercase());
        let key = DiscoveryKey::from_slice(&[7; 32]).expect("32-byte key");
        let _ = beacon.matches(&key);
    }
}

fuzz_target!(|data: &[u8]| {
    // As announced text, and as well-formed base32 so the 16-byte layout gets explored.
    if let Ok(text) = std::str::from_utf8(data) {
        check(text);
    }
    check(&base32(data));
});
