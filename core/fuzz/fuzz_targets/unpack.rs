//! The pack reader never panics, and only hands out files that match the manifest.
#![no_main]

use beam_core::pack::Unpacker;
use beam_core_fuzz::{manifest, split};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let manifest = manifest();
    let mut unpacker = Unpacker::new(&manifest);
    for piece in split(data) {
        let Ok(files) = unpacker.feed(piece) else {
            return;
        };
        for file in files {
            let item = manifest.item(file.item_id).expect("only known items");
            assert_eq!(file.data.len() as u64, item.size);
        }
    }
    let _ = unpacker.finish();
});
