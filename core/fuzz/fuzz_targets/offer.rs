//! Offered manifests, in any number of pieces, never panic, and anything accepted re-encodes
//! to a manifest that decodes to the same thing.
#![no_main]

use beam_core::offer::{OfferAssembler, encode_offer};
use beam_core_fuzz::split;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let mut assembler = OfferAssembler::new();
    for piece in split(data) {
        match assembler.push(piece) {
            Ok(Some(manifest)) => {
                let mut again = OfferAssembler::new();
                let mut back = None;
                for frame in encode_offer(&manifest, 0).expect("accepted manifests re-encode") {
                    back = again
                        .push(&frame.payload)
                        .expect("re-encoded manifests decode");
                }
                assert_eq!(back.as_ref(), Some(&manifest));
                return;
            }
            Ok(None) => {}
            Err(_) => return,
        }
    }
});
