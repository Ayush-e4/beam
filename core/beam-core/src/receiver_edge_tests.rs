use std::fs;

use tempfile::TempDir;

use super::tests::{chunks_for, fixture, item, pack_piece};
use super::*;
use crate::compress::ChunkEncoding;
use crate::manifest::TransferId;

#[test]
fn existing_files_are_never_overwritten() {
    let (f, dir) = (fixture(), TempDir::new().unwrap());
    let existing = f.manifest.item(3).unwrap().path.resolve_in(dir.path());
    fs::create_dir_all(existing.parent().unwrap()).unwrap();
    fs::write(&existing, b"mine").unwrap();
    let mut r = Receiver::open(f.manifest.clone(), dir.path(), &[3]).unwrap();
    r.handle_chunk(&pack_piece(&f, 0, &[3])).unwrap();
    let paths = r.finish().unwrap();
    assert_eq!(fs::read(&existing).unwrap(), b"mine");
    assert!(paths[0].ends_with("small/a (1).txt"));
}

#[test]
fn protocol_violations_are_rejected() {
    let (f, dir) = (fixture(), TempDir::new().unwrap());
    let mut r = Receiver::open(f.manifest.clone(), dir.path(), &[]).unwrap();
    let small_as_chunk = ChunkHeader {
        item_id: 3,
        index: 0,
        encoding: ChunkEncoding::Raw,
        raw_len: 10,
        hash: chunk_hash(&f.contents[&3]),
        root: None,
    };
    assert!(
        r.handle_chunk(&small_as_chunk.encode(&f.contents[&3]))
            .is_err()
    );
    assert!(
        r.handle_chunk(&pack_piece(&f, 1, &[3])).is_err(),
        "pack out of order"
    );
    let first = chunks_for(&f, 1).swap_remove(0);
    let (mut header, body) = ChunkHeader::decode(&first).unwrap();
    header.root = Some([1; 32]);
    assert!(
        r.handle_chunk(&header.encode(body)).is_err(),
        "root on a middle chunk"
    );
    assert!(Receiver::open(f.manifest.clone(), dir.path(), &[99]).is_err());
}

#[test]
fn finishing_early_fails_and_a_staging_path_is_refused() {
    let (f, dir) = (fixture(), TempDir::new().unwrap());
    let r = Receiver::open(f.manifest.clone(), dir.path(), &[]).unwrap();
    assert!(r.finish().is_err());

    let sneaky = vec![item(1, ".beam-partial/x", 1)];
    let manifest = Manifest::new(TransferId([2; 16]), sneaky, 1).unwrap();
    assert!(Receiver::open(manifest, dir.path(), &[]).is_err());
}

#[test]
fn it_reports_when_all_data_is_in_before_the_checkpoint() {
    let (f, dir) = (fixture(), TempDir::new().unwrap());
    let mut r = Receiver::open(f.manifest.clone(), dir.path(), &[]).unwrap();
    assert!(r.accepted_items().is_empty(), "everything selected");
    assert!(!r.has_all_data());
    for id in [1, 2] {
        for payload in chunks_for(&f, id) {
            r.handle_chunk(&payload).unwrap();
        }
    }
    assert!(!r.has_all_data(), "the pack is still missing");
    r.handle_chunk(&pack_piece(&f, 0, &[3, 4, 5])).unwrap();
    assert!(r.has_all_data());
    assert!(!r.is_complete(), "not durable until the checkpoint");
    r.checkpoint().unwrap();
    assert!(r.is_complete());
}

#[test]
fn a_partial_selection_is_listed_in_the_accept() {
    let (f, dir) = (fixture(), TempDir::new().unwrap());
    let r = Receiver::open(f.manifest.clone(), dir.path(), &[5, 3]).unwrap();
    assert_eq!(r.accepted_items(), [3, 5]);
}
