//! Opening a corrupted or hostile journal file never panics and never claims chunks the
//! manifest does not have.
#![no_main]

use std::path::PathBuf;
use std::sync::OnceLock;

use beam_core::Journal;
use beam_core_fuzz::manifest;
use libfuzzer_sys::fuzz_target;

fn journal_path() -> &'static PathBuf {
    static PATH: OnceLock<PathBuf> = OnceLock::new();
    PATH.get_or_init(|| {
        std::env::temp_dir().join(format!("beam-fuzz-journal-{}", std::process::id()))
    })
}

fuzz_target!(|data: &[u8]| {
    let manifest = manifest();
    let path = journal_path();
    std::fs::write(path, data).expect("temp file is writable");
    if let Ok(journal) = Journal::open(path, &manifest) {
        for item in manifest.items() {
            let chunks = item.layout().expect("valid layout").chunk_count();
            if let Some(done) = journal.progress(item.item_id) {
                assert_eq!(done.len(), chunks);
                assert!(done.count() <= chunks);
            }
        }
    }
});
