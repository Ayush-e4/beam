// Fixture sizes are a few hundred KiB, so the casts between u64, usize and u32 cannot truncate.
#![allow(clippy::cast_possible_truncation)]

use std::fs;

use tempfile::TempDir;

use super::*;
use crate::compress::{ChunkEncoding, encode_chunk};
use crate::integrity::hash_reader;
use crate::manifest::{MIN_CHUNK_SIZE, TransferId};
use crate::pack::Packer;
use crate::safe_path::SafePath;

pub(super) const CHUNK: u32 = MIN_CHUNK_SIZE;

pub(super) struct Fixture {
    pub(super) manifest: Manifest,
    pub(super) contents: HashMap<u32, Vec<u8>>,
}

pub(super) fn item(item_id: u32, path: &str, size: u64) -> ManifestItem {
    ManifestItem {
        item_id,
        path: SafePath::parse(path).unwrap(),
        size,
        mime: "text/plain".into(),
        modified_at_ms: Some(1_700_000_000_000),
        chunk_size: CHUNK,
    }
}

/// A large text file (3.5 chunks), a large noisy file, and small files including an empty one.
pub(super) fn fixture() -> Fixture {
    let items = vec![
        item(1, "big/notes.txt", u64::from(CHUNK) * 7 / 2),
        item(2, "big/noise.bin", u64::from(CHUNK) * 2),
        item(3, "small/a.txt", 10),
        item(4, "small/empty.txt", 0),
        item(5, "small/deeper/b.txt", 5000),
    ];
    let contents = items
        .iter()
        .map(|i| {
            let data = (0..i.size)
                .map(|n| {
                    if i.item_id == 2 {
                        chunk_hash(&n.to_le_bytes())[0]
                    } else {
                        b"beam "[(n % 5) as usize]
                    }
                })
                .collect();
            (i.item_id, data)
        })
        .collect();
    let total = items.iter().map(|i| i.size).sum();
    Fixture {
        manifest: Manifest::new(TransferId([1; 16]), items, total).unwrap(),
        contents,
    }
}

/// Every `CHUNK` payload for one large item, as the sender would produce them.
pub(super) fn chunks_for(f: &Fixture, item_id: u32) -> Vec<Vec<u8>> {
    let item = f.manifest.item(item_id).unwrap();
    let data = &f.contents[&item_id];
    let layout = item.layout().unwrap();
    let hashes = hash_reader(data.as_slice(), layout).unwrap();
    (0..layout.chunk_count())
        .map(|index| {
            let r = layout.range(index).unwrap();
            let raw = &data[r.start as usize..r.end as usize];
            let encoded = encode_chunk(raw, true).unwrap();
            let is_last = index + 1 == layout.chunk_count();
            ChunkHeader {
                item_id,
                index,
                encoding: encoded.encoding,
                raw_len: raw.len() as u32,
                hash: hashes.chunks[index as usize],
                root: is_last.then_some(hashes.root),
            }
            .encode(&encoded.bytes)
        })
        .collect()
}

/// One pack piece holding the given small items.
pub(super) fn pack_piece(f: &Fixture, index: u64, items: &[u32]) -> Vec<u8> {
    let mut packer = Packer::new();
    for id in items {
        packer
            .push(f.manifest.item(*id).unwrap(), &f.contents[id])
            .unwrap();
    }
    let raw = packer.take();
    ChunkHeader {
        item_id: PACK_ITEM_ID,
        index,
        encoding: ChunkEncoding::Raw,
        raw_len: raw.len() as u32,
        hash: chunk_hash(&raw),
        root: None,
    }
    .encode(&raw)
}

fn send_all(f: &Fixture, r: &mut Receiver) {
    for id in [1, 2] {
        for payload in chunks_for(f, id) {
            r.handle_chunk(&payload).unwrap();
        }
    }
    r.handle_chunk(&pack_piece(f, 0, &[3, 5])).unwrap();
}

pub(super) fn assert_published(f: &Fixture, dest: &Path, ids: &[u32]) {
    for id in ids {
        let item = f.manifest.item(*id).unwrap();
        assert_eq!(
            fs::read(item.path.resolve_in(dest)).unwrap(),
            f.contents[id],
            "item {id}"
        );
    }
    assert!(!dest.join(STAGING_DIR).exists(), "staging is cleaned up");
}

#[test]
fn a_full_transfer_lands_every_file_verified() {
    let (f, dir) = (fixture(), TempDir::new().unwrap());
    let mut r = Receiver::open(f.manifest.clone(), dir.path(), &[]).unwrap();
    assert!(!r.resumed());
    send_all(&f, &mut r);
    let acked = r.checkpoint().unwrap();
    assert!(acked.contains(&ChunkRange {
        item_id: 1,
        chunks: 0..4
    }));
    assert!(r.is_complete());
    let paths = r.finish().unwrap();
    assert_eq!(paths.len(), 5);
    assert_published(&f, dir.path(), &[1, 2, 3, 4, 5]);
}

#[test]
fn an_interrupted_transfer_resumes_from_what_was_acknowledged() {
    let (f, dir) = (fixture(), TempDir::new().unwrap());
    let big = chunks_for(&f, 1);
    {
        let mut r = Receiver::open(f.manifest.clone(), dir.path(), &[]).unwrap();
        r.handle_chunk(&big[0]).unwrap();
        r.handle_chunk(&big[1]).unwrap();
        r.handle_chunk(&pack_piece(&f, 0, &[3])).unwrap();
        r.checkpoint().unwrap();
        r.handle_chunk(&big[2]).unwrap(); // written but never acknowledged: the app dies here
    }
    let mut r = Receiver::open(f.manifest.clone(), dir.path(), &[]).unwrap();
    assert!(r.resumed());
    let have = r.have();
    assert!(have.contains(&ChunkRange {
        item_id: 1,
        chunks: 0..2
    }));
    assert!(have.contains(&ChunkRange {
        item_id: 3,
        chunks: 0..1
    }));
    assert!(!have.iter().any(|h| h.item_id == 1 && h.chunks.contains(&2)));

    // The sender sends only what is missing; the pack restarts at piece 0.
    for payload in &big[2..] {
        r.handle_chunk(payload).unwrap();
    }
    for payload in chunks_for(&f, 2) {
        r.handle_chunk(&payload).unwrap();
    }
    r.handle_chunk(&pack_piece(&f, 0, &[5])).unwrap();
    r.finish().unwrap();
    assert_published(&f, dir.path(), &[1, 2, 3, 4, 5]);
}

#[test]
fn resent_chunks_are_ignored_and_a_resent_small_file_is_refused() {
    let (f, dir) = (fixture(), TempDir::new().unwrap());
    let mut r = Receiver::open(f.manifest.clone(), dir.path(), &[]).unwrap();
    let big = chunks_for(&f, 1);
    r.handle_chunk(&big[0]).unwrap();
    r.checkpoint().unwrap();
    r.handle_chunk(&big[0]).unwrap();
    r.handle_chunk(&pack_piece(&f, 0, &[3])).unwrap();
    assert!(r.handle_chunk(&pack_piece(&f, 1, &[3])).is_err());
}

#[test]
fn tampered_data_never_reaches_the_disk() {
    let (f, dir) = (fixture(), TempDir::new().unwrap());
    let mut r = Receiver::open(f.manifest.clone(), dir.path(), &[]).unwrap();
    let mut chunk = chunks_for(&f, 2)[0].clone();
    let last = chunk.len() - 1;
    chunk[last] ^= 1;
    assert!(matches!(
        r.handle_chunk(&chunk),
        Err(BeamError::IntegrityMismatch { .. })
    ));
    assert_eq!(r.checkpoint().unwrap(), Vec::<ChunkRange>::new());
}

#[test]
fn a_wrong_root_hash_fails_the_file_at_finish() {
    let (f, dir) = (fixture(), TempDir::new().unwrap());
    let mut r = Receiver::open(f.manifest.clone(), dir.path(), &[1]).unwrap();
    let mut chunks = chunks_for(&f, 1);
    let last = chunks.pop().unwrap();
    let (mut header, body) = ChunkHeader::decode(&last).unwrap();
    header.root = Some([0; 32]);
    chunks.push(header.encode(body));
    for payload in &chunks {
        r.handle_chunk(payload).unwrap();
    }
    assert!(matches!(
        r.finish(),
        Err(BeamError::IntegrityMismatch { .. })
    ));
    assert!(
        !f.manifest
            .item(1)
            .unwrap()
            .path
            .resolve_in(dir.path())
            .exists()
    );
}

#[test]
fn only_selected_items_are_accepted_and_published() {
    let (f, dir) = (fixture(), TempDir::new().unwrap());
    let mut r = Receiver::open(f.manifest.clone(), dir.path(), &[2, 3]).unwrap();
    assert!(r.handle_chunk(&chunks_for(&f, 1)[0]).is_err());
    assert!(r.handle_chunk(&pack_piece(&f, 0, &[5])).is_err());
    let mut r = Receiver::open(f.manifest.clone(), dir.path(), &[2, 3]).unwrap();
    for payload in chunks_for(&f, 2) {
        r.handle_chunk(&payload).unwrap();
    }
    r.handle_chunk(&pack_piece(&f, 0, &[3])).unwrap();
    assert_eq!(r.finish().unwrap().len(), 2);
    assert_published(&f, dir.path(), &[2, 3]);
    assert!(
        !f.manifest
            .item(1)
            .unwrap()
            .path
            .resolve_in(dir.path())
            .exists()
    );
}
