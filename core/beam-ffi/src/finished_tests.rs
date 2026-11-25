// Test code reports a broken setup by panicking.
#![allow(clippy::unwrap_used)]

use std::fs;

use tempfile::TempDir;

use super::*;

fn id(n: u32) -> TransferId {
    let mut bytes = [0u8; 16];
    bytes[..4].copy_from_slice(&n.to_le_bytes());
    TransferId(bytes)
}

#[test]
fn the_list_survives_a_restart_once_it_has_a_file() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("finished.bin");
    let before = Finished::default();
    before.insert(id(1)).unwrap(); // before the app named a file: memory only
    before.keep_in(path.clone()).unwrap();
    before.insert(id(2)).unwrap();

    let after = Finished::default();
    after.keep_in(path).unwrap();
    assert!(after.contains(id(1)) && after.contains(id(2)));
    assert!(!after.contains(id(3)));
}

#[test]
fn only_the_newest_are_remembered() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("finished.bin");
    let finished = Finished::default();
    finished.keep_in(path.clone()).unwrap();
    let count = u32::try_from(REMEMBERED).unwrap() + 10;
    for n in 0..count {
        finished.insert(id(n)).unwrap();
    }
    finished.insert(id(count - 1)).unwrap(); // a repeat changes nothing

    let reloaded = Finished::default();
    reloaded.keep_in(path).unwrap();
    assert!(!reloaded.contains(id(9)));
    assert!(reloaded.contains(id(10)) && reloaded.contains(id(count - 1)));
}

#[test]
fn a_damaged_file_is_an_error() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("finished.bin");
    for bytes in [&b"BEAMFIN1short"[..], &b"something else entirely"[..]] {
        fs::write(&path, bytes).unwrap();
        let error = Finished::default().keep_in(path.clone()).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    }
}
