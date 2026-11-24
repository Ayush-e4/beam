//! Frame decoding never panics, and anything it accepts re-encodes to the same bytes.
#![no_main]

use beam_core::{Decoded, Frame};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(Decoded::Frame { frame, consumed }) = Frame::decode(data) {
        let mut encoded = Vec::new();
        frame
            .encode_into(&mut encoded)
            .expect("decoded frames re-encode");
        assert_eq!(encoded, data[..consumed]);
    }
});
