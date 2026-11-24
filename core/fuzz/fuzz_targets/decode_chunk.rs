//! Decompression never panics and never produces more than the declared chunk size.
#![no_main]

use beam_core::compress::{ChunkEncoding, decode_chunk};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Some((len_bytes, body)) = data.split_first_chunk::<2>() else {
        return;
    };
    let expected = usize::from(u16::from_le_bytes(*len_bytes)) * 16;
    if let Ok(out) = decode_chunk(ChunkEncoding::Zstd, body, expected) {
        assert_eq!(out.len(), expected);
    }
});
