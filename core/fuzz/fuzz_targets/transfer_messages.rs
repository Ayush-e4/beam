//! Transfer message decoders never panic, and accepted messages re-encode identically.
#![no_main]

use beam_core::messages::{
    Accept, ChunkHeader, decode_ack, decode_reason, decode_resume, encode_ack, encode_reason,
    encode_resume,
};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(accept) = Accept::decode(data) {
        assert_eq!(accept.encode().expect("re-encodes"), data);
    }
    if let Ok((id, reason)) = decode_reason(data) {
        assert_eq!(encode_reason(id, reason), data);
    }
    if let Ok((id, ranges)) = decode_resume(data) {
        assert_eq!(encode_resume(id, &ranges).expect("re-encodes"), data);
    }
    if let Ok(ranges) = decode_ack(data) {
        assert_eq!(encode_ack(&ranges).expect("re-encodes"), data);
    }
    if let Ok((header, body)) = ChunkHeader::decode(data) {
        assert_eq!(header.encode(body), data);
    }
});
