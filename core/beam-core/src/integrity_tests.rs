use std::io::Cursor;

use proptest::prelude::*;

use super::*;

const SMALL_CHUNK: u32 = 4;

fn layout(size: u64) -> ChunkLayout {
    ChunkLayout::new(size, SMALL_CHUNK).expect("valid layout")
}

#[test]
fn splits_into_full_chunks_and_a_short_tail() {
    let layout = layout(10);
    assert_eq!(layout.chunk_count(), 3);
    assert_eq!(layout.range(0).expect("in range"), 0..4);
    assert_eq!(layout.range(2).expect("in range"), 8..10);
    assert!(layout.range(3).is_err());
}

#[test]
fn empty_files_have_no_chunks() {
    assert_eq!(layout(0).chunk_count(), 0);
    assert!(layout(0).range(0).is_err());
}

#[test]
fn zero_chunk_size_is_rejected() {
    assert!(ChunkLayout::new(10, 0).is_err());
}

#[test]
fn verifies_a_correct_chunk() {
    let data = b"abcd";
    layout(8)
        .verify_chunk(0, data, &chunk_hash(data))
        .expect("chunk is valid");
}

#[test]
fn detects_tampered_chunks() {
    let err = layout(8)
        .verify_chunk(1, b"abcd", &chunk_hash(b"abce"))
        .expect_err("tampered");
    assert!(matches!(err, BeamError::IntegrityMismatch { chunk: 1 }));
    assert!(is_integrity_error(&err));
}

#[test]
fn rejects_chunks_of_the_wrong_length() {
    let err = layout(10)
        .verify_chunk(2, b"abc", &chunk_hash(b"abc"))
        .expect_err("too long");
    assert_eq!(err.category(), ErrorCategory::Protocol);
}

#[test]
fn chunk_and_root_hashes_use_separate_domains() {
    let data = [0u8; 32];
    let mut root = RootHasher::new(layout(0));
    assert_ne!(chunk_hash(&data), root.finalize().expect("empty file"));
    root.push(&data);
    assert!(
        root.finalize().is_err(),
        "too many chunks for an empty file"
    );
}

#[test]
fn hash_reader_matches_manual_hashing() {
    let data = b"0123456789";
    let hashes = hash_reader(Cursor::new(data), layout(10)).expect("hashes");

    let expected: Vec<Hash> = [&data[..4], &data[4..8], &data[8..]]
        .map(chunk_hash)
        .to_vec();
    assert_eq!(hashes.chunks, expected);

    let mut root = RootHasher::new(layout(10));
    for hash in &expected {
        root.push(hash);
    }
    assert_eq!(hashes.root, root.finalize().expect("complete"));
}

#[test]
fn hash_reader_rejects_size_mismatches() {
    assert!(hash_reader(Cursor::new(b"short"), layout(10)).is_err());
    assert!(hash_reader(Cursor::new(b"too long"), layout(4)).is_err());
}

#[test]
fn root_depends_on_the_layout() {
    let data = b"abcdefgh";
    let small = hash_reader(Cursor::new(data), layout(8)).expect("hashes");
    let large =
        hash_reader(Cursor::new(data), ChunkLayout::new(8, 8).expect("valid")).expect("hashes");
    assert_ne!(small.root, large.root);
}

proptest! {
    #[test]
    fn every_chunk_of_a_file_verifies(data in proptest::collection::vec(any::<u8>(), 0..200)) {
        let size = data.len() as u64;
        let layout = layout(size);
        let hashes = hash_reader(Cursor::new(&data), layout).expect("hashes");
        prop_assert_eq!(hashes.chunks.len() as u64, layout.chunk_count());

        for (index, hash) in hashes.chunks.iter().enumerate() {
            let range = layout.range(index as u64).expect("in range");
            let start = usize::try_from(range.start).expect("fits");
            let end = usize::try_from(range.end).expect("fits");
            let chunk = &data[start..end];
            prop_assert!(layout.verify_chunk(index as u64, chunk, hash).is_ok());
        }
    }
}
