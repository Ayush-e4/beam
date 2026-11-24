use super::*;
use crate::ErrorCategory;

const ID: TransferId = TransferId([7; 16]);

fn item(item_id: u32, path: &str, size: u64) -> ManifestItem {
    ManifestItem {
        item_id,
        path: SafePath::parse(path).expect("valid path"),
        size,
        mime: "application/octet-stream".into(),
        modified_at_ms: None,
        chunk_size: 4 * 1024 * 1024,
        root_hash: [0; 32],
    }
}

fn is_malformed(result: &Result<Manifest>) -> bool {
    matches!(result, Err(err) if err.category() == ErrorCategory::Protocol)
}

#[test]
fn builds_a_valid_manifest() {
    let manifest = Manifest::new(ID, vec![item(1, "a.txt", 10), item(2, "dir/b.txt", 5)], 15)
        .expect("valid manifest");

    assert_eq!(manifest.transfer_id(), ID);
    assert_eq!(manifest.total_size(), 15);
    assert_eq!(manifest.items().len(), 2);
    assert_eq!(
        manifest.item(2).map(|i| i.path.to_string()),
        Some("dir/b.txt".into())
    );
    assert!(manifest.item(3).is_none());
}

#[test]
fn rejects_empty_manifests() {
    assert!(is_malformed(&Manifest::new(ID, Vec::new(), 0)));
}

#[test]
fn rejects_a_wrong_declared_total() {
    assert!(is_malformed(&Manifest::new(ID, vec![item(1, "a", 10)], 11)));
}

#[test]
fn rejects_duplicate_ids() {
    let items = vec![item(1, "a", 1), item(1, "b", 1)];
    assert!(is_malformed(&Manifest::new(ID, items, 2)));
}

#[test]
fn rejects_paths_that_collide_ignoring_case() {
    let items = vec![item(1, "Docs/Report.pdf", 1), item(2, "docs/report.PDF", 1)];
    assert!(is_malformed(&Manifest::new(ID, items, 2)));
}

#[test]
fn rejects_a_file_that_is_also_a_folder() {
    let items = vec![item(1, "photos", 1), item(2, "Photos/cat.jpg", 1)];
    assert!(is_malformed(&Manifest::new(ID, items, 2)));
}

#[test]
fn rejects_invalid_chunk_sizes() {
    for chunk_size in [0, MIN_CHUNK_SIZE - 1, MAX_CHUNK_SIZE + 1] {
        let mut bad = item(1, "a", 1);
        bad.chunk_size = chunk_size;
        assert!(
            is_malformed(&Manifest::new(ID, vec![bad], 1)),
            "chunk size {chunk_size}"
        );
    }
}

#[test]
fn rejects_sizes_that_overflow() {
    let items = vec![item(1, "a", u64::MAX), item(2, "b", 1)];
    assert!(is_malformed(&Manifest::new(ID, items, 0)));
}

#[test]
fn counts_chunks_rounding_up() {
    let mut file = item(1, "a", 0);
    assert_eq!(file.chunk_count(), 0);
    file.size = 1;
    assert_eq!(file.chunk_count(), 1);
    file.size = u64::from(file.chunk_size) * 3 + 1;
    assert_eq!(file.chunk_count(), 4);
}
