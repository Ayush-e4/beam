use proptest::prelude::*;

use super::*;
use crate::SafePath;
use crate::manifest::{MIN_CHUNK_SIZE, TransferId};

const CHUNK: u32 = 4 * 1024 * 1024;

fn item(item_id: u32, size: u64, chunk_size: u32) -> ManifestItem {
    ManifestItem {
        item_id,
        path: SafePath::parse(&format!("dir/file-{item_id}.txt")).unwrap(),
        size,
        mime: "text/plain".into(),
        modified_at_ms: None,
        chunk_size,
    }
}

fn contents(item: &ManifestItem) -> Vec<u8> {
    let seed = item.item_id.to_le_bytes()[0];
    (0..=255u8)
        .cycle()
        .map(|b| b ^ seed)
        .take(usize::try_from(item.size).unwrap())
        .collect()
}

fn manifest(items: Vec<ManifestItem>) -> Manifest {
    let total = items.iter().map(|i| i.size).sum();
    Manifest::new(TransferId([3; 16]), items, total).unwrap()
}

/// Small files 1..=n with varied sizes, including an empty one, plus one large file.
fn mixed(n: u32) -> Manifest {
    let mut items: Vec<_> = (1..=n)
        .map(|id| item(id, u64::from(id * 977 % 5000), CHUNK))
        .collect();
    items.push(item(n + 1, u64::from(MAX_PACKED_FILE) + 1, CHUNK));
    manifest(items)
}

fn packed(manifest: &Manifest) -> Vec<u8> {
    let mut packer = Packer::new();
    for item in manifest.items().iter().filter(|i| is_packable(i)) {
        packer.push(item, &contents(item)).unwrap();
    }
    packer.take()
}

#[test]
fn packability_follows_size_and_chunk_size() {
    assert!(is_packable(&item(1, 0, CHUNK)));
    assert!(is_packable(&item(1, u64::from(MAX_PACKED_FILE), CHUNK)));
    assert!(!is_packable(&item(
        1,
        u64::from(MAX_PACKED_FILE) + 1,
        CHUNK
    )));
    // A file larger than its own chunk would not be a single chunk, so it is not packed.
    let small_chunks = item(1, u64::from(MIN_CHUNK_SIZE) + 1, MIN_CHUNK_SIZE);
    assert!(!is_packable(&small_chunks));
}

#[test]
fn every_small_file_comes_back_verified() {
    let manifest = mixed(300);
    let wire = packed(&manifest);
    let mut unpacker = Unpacker::new(&manifest);
    assert_eq!(unpacker.remaining(), 300);
    let files = unpacker.feed(&wire).unwrap();
    unpacker.finish().unwrap();
    assert_eq!(files.len(), 300);
    assert_eq!(unpacker.remaining(), 0);
    for file in files {
        let item = manifest.item(file.item_id).unwrap();
        assert_eq!(file.data, contents(item));
        assert_eq!(file.hash, chunk_hash(&file.data));
    }
}

#[test]
fn overhead_is_a_fixed_forty_bytes_per_file() {
    let manifest = mixed(50);
    let data: usize = manifest.items()[..50]
        .iter()
        .map(|i| usize::try_from(i.size).unwrap())
        .sum();
    assert_eq!(packed(&manifest).len(), data + 50 * ENTRY_OVERHEAD);
    assert_eq!(ENTRY_OVERHEAD, 40);
}

#[test]
fn packer_refuses_large_or_changed_files() {
    let big = item(1, u64::from(MAX_PACKED_FILE) + 1, CHUNK);
    let small = item(2, 10, CHUNK);
    let mut packer = Packer::new();
    assert!(packer.push(&big, &contents(&big)).is_err());
    assert!(packer.push(&small, &[0; 11]).is_err());
    assert_eq!(packer.pending(), 0);
    packer.push(&small, &contents(&small)).unwrap();
    assert_eq!(packer.pending(), 10 + ENTRY_OVERHEAD);
}

#[test]
fn corrupted_file_fails_its_hash() {
    let manifest = mixed(3);
    let mut wire = packed(&manifest);
    wire[HEADER_LEN] ^= 1;
    let err = Unpacker::new(&manifest).feed(&wire).unwrap_err();
    assert!(matches!(err, BeamError::IntegrityMismatch { .. }));
}

#[test]
fn unknown_repeated_large_or_resized_entries_are_rejected() {
    let manifest = mixed(3);
    let entry = |id: u32, len: u32| {
        let mut e = id.to_le_bytes().to_vec();
        e.extend(len.to_le_bytes());
        e
    };
    let large_id = 4;
    for header in [entry(99, 0), entry(large_id, 10), entry(2, 1)] {
        assert!(matches!(
            Unpacker::new(&manifest).feed(&header),
            Err(BeamError::Malformed(_))
        ));
    }
    let wire = packed(&manifest);
    let mut unpacker = Unpacker::new(&manifest);
    unpacker.feed(&wire).unwrap();
    assert!(
        unpacker.feed(&wire).is_err(),
        "a second copy must be refused"
    );
}

#[test]
fn files_already_in_the_journal_are_not_expected_again() {
    let manifest = mixed(3);
    let mut unpacker = Unpacker::new(&manifest);
    unpacker.skip(2);
    assert_eq!(unpacker.remaining(), 2);
    assert!(unpacker.feed(&packed(&manifest)).is_err());
}

#[test]
fn a_stream_cut_mid_entry_does_not_finish_cleanly() {
    let manifest = mixed(3);
    let wire = packed(&manifest);
    let mut unpacker = Unpacker::new(&manifest);
    unpacker.feed(&wire[..wire.len() - 5]).unwrap();
    assert!(unpacker.finish().is_err());
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    #[test]
    fn any_split_recovers_the_same_files(cuts in prop::collection::vec(1usize..4000, 0..40)) {
        let manifest = mixed(40);
        let wire = packed(&manifest);
        let mut unpacker = Unpacker::new(&manifest);
        let mut got = Vec::new();
        let mut rest = wire.as_slice();
        for cut in cuts {
            let (head, tail) = rest.split_at(cut.min(rest.len()));
            got.extend(unpacker.feed(head).unwrap());
            rest = tail;
        }
        got.extend(unpacker.feed(rest).unwrap());
        unpacker.finish().unwrap();
        let ids: Vec<u32> = got.iter().map(|f| f.item_id).collect();
        prop_assert_eq!(ids, (1..=40).collect::<Vec<_>>());
    }
}
