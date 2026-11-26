//! Files that live only behind [`ReadAt`] (as a browser's picked files do), sent to the real
//! receiver in-process.

use std::fs;
use std::io;
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use tempfile::TempDir;

use super::tests::send_all;
use super::*;
use crate::messages::Accept;
use crate::{BeamError, Sender};

const ID: TransferId = TransferId([6; 16]);

/// A file held in memory, which tests can change or break under the sender.
#[derive(Debug)]
struct MemoryFile {
    bytes: Mutex<Vec<u8>>,
    modified: SystemTime,
    broken: bool,
}

impl MemoryFile {
    fn new(bytes: &[u8]) -> Arc<Self> {
        Arc::new(Self {
            bytes: Mutex::new(bytes.to_vec()),
            modified: UNIX_EPOCH + Duration::from_secs(1_764_000_007),
            broken: false,
        })
    }
}

impl ReadAt for MemoryFile {
    fn size(&self) -> io::Result<u64> {
        Ok(self.bytes.lock().unwrap().len() as u64)
    }

    fn modified(&self) -> io::Result<Option<SystemTime>> {
        Ok(Some(self.modified))
    }

    fn read_exact_at(&self, buf: &mut [u8], offset: u64) -> io::Result<()> {
        if self.broken {
            return Err(io::Error::other("the browser lost the file"));
        }
        let bytes = self.bytes.lock().unwrap();
        let start = usize::try_from(offset).unwrap();
        let end = start + buf.len();
        let Some(slice) = bytes.get(start..end) else {
            return Err(io::ErrorKind::UnexpectedEof.into());
        };
        buf.copy_from_slice(slice);
        Ok(())
    }
}

fn readable(name: &str, file: Arc<MemoryFile>) -> ReadableFile {
    ReadableFile {
        name: name.to_owned(),
        reader: file,
    }
}

/// Sends until the sender fails, and returns the error.
fn send_error(prepared: PreparedTransfer) -> BeamError {
    let mut sender = Sender::new(prepared);
    sender
        .accept(&Accept {
            transfer_id: ID,
            items: Vec::new(),
        })
        .unwrap();
    loop {
        match sender.next_chunk() {
            Ok(Some(_)) => {}
            Ok(None) => panic!("the transfer finished without noticing"),
            Err(err) => return err,
        }
    }
}

#[test]
fn files_behind_read_at_arrive_intact_under_their_names() {
    let big: Vec<u8> = (0..9_000_000u32).map(|n| (n % 251) as u8).collect();
    let files = vec![
        readable("video.mp4", MemoryFile::new(&big)),
        readable("video.mp4", MemoryFile::new(b"same name, other file")),
        readable("empty.txt", MemoryFile::new(b"")),
    ];
    let prepared = prepare_readable_with_id(files, ID).unwrap();
    let item = &prepared.manifest().items()[0];
    assert_eq!(item.size, 9_000_000);
    assert_eq!(item.modified_at_ms, Some(1_764_000_007_000));

    let dir = TempDir::new().unwrap();
    send_all(prepared, dir.path());
    assert_eq!(fs::read(dir.path().join("video.mp4")).unwrap(), big);
    assert_eq!(
        fs::read(dir.path().join("video (2).mp4")).unwrap(),
        b"same name, other file"
    );
    assert_eq!(fs::read(dir.path().join("empty.txt")).unwrap(), b"");
}

#[test]
fn a_file_that_changes_after_the_offer_is_caught() {
    let file = MemoryFile::new(&vec![7; 3_000_000]);
    let prepared = prepare_readable_with_id(vec![readable("draft.txt", file.clone())], ID).unwrap();
    file.bytes.lock().unwrap().truncate(1_000);

    let err = send_error(prepared);
    assert!(
        matches!(err, BeamError::SourceChanged { ref path } if path == "draft.txt"),
        "{err:?}"
    );
}

#[test]
fn a_file_that_cannot_be_read_fails_as_storage_naming_it() {
    let file = Arc::new(MemoryFile {
        broken: true,
        ..Arc::try_unwrap(MemoryFile::new(&vec![1; 3_000_000])).unwrap()
    });
    let prepared = prepare_readable_with_id(vec![readable("photo.jpg", file)], ID).unwrap();

    let err = send_error(prepared);
    assert_eq!(err.category(), crate::ErrorCategory::Storage, "{err:?}");
    assert!(err.to_string().contains("photo.jpg"), "{err}");
}
