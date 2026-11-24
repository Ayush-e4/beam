use std::fs::OpenOptions;
use std::io::Write;

use tempfile::TempDir;

use super::*;
use crate::SafePath;
use crate::manifest::{MIN_CHUNK_SIZE, ManifestItem, TransferId};

fn manifest(transfer: u8) -> Manifest {
    let chunk = u64::from(MIN_CHUNK_SIZE);
    let item = |item_id, path: &str, size| ManifestItem {
        item_id,
        path: SafePath::parse(path).expect("valid path"),
        size,
        mime: "application/octet-stream".into(),
        modified_at_ms: None,
        chunk_size: MIN_CHUNK_SIZE,
    };
    let items = vec![
        item(1, "video.mp4", chunk * 10),
        item(2, "notes.txt", chunk),
    ];
    Manifest::new(TransferId([transfer; 16]), items, chunk * 11).expect("valid manifest")
}

fn journal_path(dir: &TempDir) -> std::path::PathBuf {
    dir.path().join("transfer.journal")
}

#[test]
fn starts_fresh_when_no_journal_exists() {
    let dir = TempDir::new().expect("tempdir");
    let journal = Journal::open(&journal_path(&dir), &manifest(1)).expect("opens");

    assert!(!journal.resumed());
    assert_eq!(journal.progress(1).map(ChunkSet::count), Some(0));
    assert!(journal.progress(9).is_none());
}

#[test]
fn recovers_recorded_chunks_after_reopening() {
    let dir = TempDir::new().expect("tempdir");
    let path = journal_path(&dir);
    let manifest = manifest(1);
    {
        let mut journal = Journal::open(&path, &manifest).expect("opens");
        journal
            .record(&[(1, 0), (1, 1), (1, 7), (2, 0)])
            .expect("records");
    }

    let journal = Journal::open(&path, &manifest).expect("reopens");
    assert!(journal.resumed());
    assert_eq!(
        journal.progress(1).expect("item").ranges(),
        vec![0..2, 7..8]
    );
    assert!(journal.progress(2).expect("item").is_complete());
    assert!(!journal.is_complete());
}

#[test]
fn cuts_off_a_torn_record_after_a_crash() {
    let dir = TempDir::new().expect("tempdir");
    let path = journal_path(&dir);
    let manifest = manifest(1);
    {
        let mut journal = Journal::open(&path, &manifest).expect("opens");
        journal.record(&[(1, 0), (1, 1)]).expect("records");
    }
    // Simulate dying half-way through appending a record.
    let mut file = OpenOptions::new().append(true).open(&path).expect("opens");
    file.write_all(&[0xAB; RECORD_LEN / 2]).expect("writes");
    drop(file);

    let mut journal = Journal::open(&path, &manifest).expect("reopens");
    assert_eq!(journal.progress(1).expect("item").count(), 2);
    journal.record(&[(1, 2)]).expect("appends after recovery");

    let journal = Journal::open(&path, &manifest).expect("reopens again");
    assert_eq!(journal.progress(1).expect("item").ranges(), vec![0..3]);
}

#[test]
fn stops_at_a_corrupted_record() {
    let dir = TempDir::new().expect("tempdir");
    let path = journal_path(&dir);
    let manifest = manifest(1);
    {
        let mut journal = Journal::open(&path, &manifest).expect("opens");
        journal.record(&[(1, 0), (1, 1), (1, 2)]).expect("records");
    }
    let mut bytes = std::fs::read(&path).expect("reads");
    bytes[HEADER_LEN + RECORD_LEN + 3] ^= 0xFF; // flip a bit in the second record
    std::fs::write(&path, bytes).expect("writes");

    let journal = Journal::open(&path, &manifest).expect("reopens");
    assert_eq!(journal.progress(1).expect("item").ranges(), vec![0..1]);
}

#[test]
fn ignores_a_journal_from_a_different_manifest() {
    let dir = TempDir::new().expect("tempdir");
    let path = journal_path(&dir);
    {
        let mut journal = Journal::open(&path, &manifest(1)).expect("opens");
        journal.record(&[(1, 0)]).expect("records");
    }

    let journal = Journal::open(&path, &manifest(2)).expect("opens for another transfer");
    assert!(!journal.resumed());
    assert_eq!(journal.progress(1).map(ChunkSet::count), Some(0));
}

#[test]
fn rejects_unknown_items_and_chunks_without_writing() {
    let dir = TempDir::new().expect("tempdir");
    let path = journal_path(&dir);
    let manifest = manifest(1);
    let mut journal = Journal::open(&path, &manifest).expect("opens");

    assert!(journal.record(&[(1, 0), (9, 0)]).is_err());
    assert!(journal.record(&[(2, 1)]).is_err());
    assert_eq!(journal.progress(1).map(ChunkSet::count), Some(0));

    let reopened = Journal::open(&path, &manifest).expect("reopens");
    assert_eq!(reopened.progress(1).map(ChunkSet::count), Some(0));
}

#[test]
fn completes_once_every_chunk_is_recorded() {
    let dir = TempDir::new().expect("tempdir");
    let mut journal = Journal::open(&journal_path(&dir), &manifest(1)).expect("opens");
    let all: Vec<(u32, u64)> = (0..10).map(|c| (1, c)).chain([(2, 0)]).collect();
    journal.record(&all).expect("records");
    assert!(journal.is_complete());
}
