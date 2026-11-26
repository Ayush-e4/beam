//! Signaling messages from the room never panic: arbitrary bytes are refused by the seal, and
//! any plain message accepted re-encodes to the same bytes.
#![no_main]

use std::sync::OnceLock;

use beam_core::signaling::{decode, encode};
use beam_core::{Cipher, PairingInvite, PublicKey, SignalFrom, SignalSeal};
use libfuzzer_sys::fuzz_target;

fn seal() -> &'static SignalSeal {
    static SEAL: OnceLock<SignalSeal> = OnceLock::new();
    SEAL.get_or_init(|| {
        let invite = PairingInvite::new(PublicKey::from_bytes([1; 32]), Cipher::AesGcm, Vec::new())
            .expect("an invite without hints");
        SignalSeal::for_invite(&invite).expect("a seal")
    })
}

fuzz_target!(|data: &[u8]| {
    if let Ok(message) = decode(data) {
        assert_eq!(encode(&message).expect("accepted messages re-encode"), data);
    }
    // Forging a seal without the key would take guessing a 128-bit tag.
    assert!(seal().open(SignalFrom::Scanner, data).is_err());
});
