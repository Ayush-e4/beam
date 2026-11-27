//! Clipboard payloads never panic, and anything accepted re-encodes to the same bytes.
#![no_main]

use beam_core::ClipItem;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(item) = ClipItem::decode(data) {
        assert_eq!(item.encode(), data, "accepted payloads re-encode unchanged");
        let _ = item.label();
    }
});
