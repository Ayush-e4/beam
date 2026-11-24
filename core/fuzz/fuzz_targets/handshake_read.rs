//! A responder fed a forged first message never panics, and garbage never completes pairing.
#![no_main]

use beam_core_fuzz::pairing_responder;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let mut responder = pairing_responder();
    if responder.read_message(data).is_ok() {
        // The first XX message is an unauthenticated ephemeral key, so it may be accepted;
        // the handshake must still be unfinished.
        assert!(!responder.is_finished());
        let _ = responder.write_message();
    }
});
