//! Hostile bytes after a real handshake are rejected without panicking, in any fragmentation.
#![no_main]

use beam_core_fuzz::{paired_sessions, split};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let (_, mut receiver) = paired_sessions();
    for piece in split(data) {
        if receiver.receive(piece).is_err() {
            assert!(
                receiver.receive(&[0]).is_err(),
                "a failed session must stay broken"
            );
            return;
        }
    }
});
