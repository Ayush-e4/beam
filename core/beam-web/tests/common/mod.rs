//! What the browser tests share: files to send, read from memory, and this worker's OPFS.
#![allow(dead_code, clippy::unwrap_used)] // each test crate uses part of it

use std::io;
use std::sync::Arc;
use std::time::SystemTime;

use beam_core::TransferStaging;
use beam_core::{Manifest, ReadAt, ReadableFile};
use beam_web::opfs_staging::layout;
use wasm_bindgen::JsCast;
use web_sys::{FileSystemDirectoryHandle, WorkerGlobalScope};

#[derive(Debug)]
pub struct Memory(pub Vec<u8>);

impl ReadAt for Memory {
    fn size(&self) -> io::Result<u64> {
        Ok(self.0.len() as u64)
    }

    fn modified(&self) -> io::Result<Option<SystemTime>> {
        Ok(None)
    }

    fn read_exact_at(&self, buf: &mut [u8], offset: u64) -> io::Result<()> {
        let start = usize::try_from(offset).unwrap();
        let bytes = self.0.get(start..start + buf.len());
        buf.copy_from_slice(bytes.ok_or(io::ErrorKind::UnexpectedEof)?);
        Ok(())
    }
}

/// Bytes that don't compress, so the wire carries every one (windows and acks count those).
fn noise(len: usize) -> Vec<u8> {
    let mut state = 0x9E37_79B9_7F4A_7C15_u64;
    (0..len)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state.to_le_bytes()[0]
        })
        .collect()
}

/// A two-chunk file (chunks are 4 MiB), a small one that gets packed, and an empty one.
pub fn files() -> Vec<(&'static str, Vec<u8>)> {
    let small = b"a small note".repeat(200);
    vec![
        ("video.mp4", noise(5_000_000)),
        ("note.txt", small),
        ("empty", Vec::new()),
    ]
}

pub fn readable() -> Vec<ReadableFile> {
    files()
        .into_iter()
        .map(|(name, data)| ReadableFile {
            name: name.to_owned(),
            reader: Arc::new(Memory(data)),
        })
        .collect()
}

/// This worker's OPFS root.
pub async fn root() -> FileSystemDirectoryHandle {
    let scope: WorkerGlobalScope = js_sys::global().unchecked_into();
    let promise = scope.navigator().storage().get_directory();
    let root = wasm_bindgen_futures::JsFuture::from(promise).await.unwrap();
    root.unchecked_into()
}

/// Every file's bytes as `staging` holds them match [`files`].
pub fn assert_intact(manifest: &Manifest, staging: &mut dyn TransferStaging) {
    for (item, (name, data)) in layout(manifest, &[]).unwrap().iter().zip(&files()) {
        assert_eq!(item.path, *name);
        let staged = staging.read_range(item.item_id, 0..item.size).unwrap();
        assert!(staged == *data, "{name} differs");
    }
}
