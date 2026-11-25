//! Scanned pairing codes never panic, and any code accepted re-encodes to an invite that reads
//! back the same.
#![no_main]

use beam_core::PairingInvite;
use beam_core::invite::INVITE_PREFIX;
use beam_core_fuzz::base32;
use libfuzzer_sys::fuzz_target;

fn check(text: &str) {
    if let Ok(invite) = PairingInvite::decode(text) {
        let again = PairingInvite::decode(&invite.encode()).expect("accepted invites re-encode");
        assert_eq!(again, invite);
    }
}

fuzz_target!(|data: &[u8]| {
    // As scanned text, and wrapped as a well-formed code so the binary layout gets explored.
    if let Ok(text) = std::str::from_utf8(data) {
        check(text);
    }
    check(&format!("{INVITE_PREFIX}{}", base32(data)));
});
