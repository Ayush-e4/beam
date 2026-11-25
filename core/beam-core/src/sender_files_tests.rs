use super::*;
use crate::ErrorCategory;
use std::fs::File;
use std::io::Write;
use tempfile::TempDir;

const ID: TransferId = TransferId([3; 16]);

fn write(path: &Path, bytes: &[u8]) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("create parent");
    }
    File::create(path)
        .and_then(|mut file| file.write_all(bytes))
        .expect("write file");
}

fn paths(prepared: &PreparedTransfer) -> Vec<String> {
    prepared
        .manifest()
        .items()
        .iter()
        .map(|item| item.path.to_string())
        .collect()
}

#[test]
fn offers_a_single_file_under_its_name() {
    let dir = TempDir::new().expect("temp dir");
    let file = dir.path().join("photo.jpg");
    write(&file, b"jpeg bytes");

    let prepared = prepare_transfer_with_id(std::slice::from_ref(&file), ID).expect("prepare");
    let manifest = prepared.manifest();
    assert_eq!(manifest.transfer_id(), ID);
    assert_eq!(manifest.total_size(), 10);

    let item = &manifest.items()[0];
    assert_eq!(item.path.to_string(), "photo.jpg");
    assert_eq!(item.size, 10);
    assert_eq!(item.mime, "image/jpeg");
    assert_eq!(item.chunk_size, DEFAULT_CHUNK_SIZE);
    assert!(item.modified_at_ms.is_some());

    let source = prepared.source(item.item_id).expect("source");
    assert_eq!(source.path, file);
    assert_eq!(source.size, 10);
}

#[test]
fn walks_folders_in_name_order_and_skips_empty_folders() {
    let dir = TempDir::new().expect("temp dir");
    let root = dir.path().join("trip");
    write(&root.join("b.txt"), b"bb");
    write(&root.join("a.txt"), b"a");
    write(&root.join("day 1/notes.md"), b"");
    fs::create_dir_all(root.join("empty/deeper")).expect("empty folders");

    let prepared = prepare_transfer_with_id(std::slice::from_ref(&root), ID).expect("prepare");
    assert_eq!(
        paths(&prepared),
        ["trip/a.txt", "trip/b.txt", "trip/day 1/notes.md"]
    );
    assert_eq!(prepared.manifest().total_size(), 3);
    let notes = prepared.source(2).expect("source");
    assert_eq!(notes.path, root.join("day 1/notes.md"));
}

#[test]
fn renames_picked_items_that_share_a_name() {
    let dir = TempDir::new().expect("temp dir");
    let first = dir.path().join("one/report.pdf");
    let second = dir.path().join("two/Report.pdf");
    write(&first, b"1");
    write(&second, b"2");

    let prepared = prepare_transfer_with_id(&[first, second], ID).expect("prepare");
    assert_eq!(paths(&prepared), ["report.pdf", "Report (2).pdf"]);
}

#[cfg(unix)]
#[test]
fn skips_symbolic_links_inside_folders_but_follows_picked_ones() {
    use std::os::unix::fs::symlink;

    let dir = TempDir::new().expect("temp dir");
    let outside = dir.path().join("secret.txt");
    write(&outside, b"secret");
    let root = dir.path().join("share");
    write(&root.join("kept.txt"), b"kept");
    symlink(&outside, root.join("link.txt")).expect("file link");
    symlink(&root, root.join("loop")).expect("folder link");
    let picked_link = dir.path().join("picked.txt");
    symlink(&outside, &picked_link).expect("picked link");

    let prepared = prepare_transfer_with_id(&[root, picked_link], ID).expect("prepare");
    assert_eq!(paths(&prepared), ["share/kept.txt", "picked.txt"]);
}

#[test]
fn refuses_a_selection_without_files() {
    let dir = TempDir::new().expect("temp dir");
    let empty = dir.path().join("empty");
    fs::create_dir(&empty).expect("empty folder");

    for selection in [Vec::new(), vec![empty]] {
        let result = prepare_transfer_with_id(&selection, ID);
        assert!(matches!(result, Err(BeamError::Selection(_))));
    }
}

#[test]
fn refuses_folders_nested_too_deeply() {
    let dir = TempDir::new().expect("temp dir");
    let mut deep = dir.path().join("top");
    for _ in 0..MAX_DEPTH {
        deep.push("d");
    }
    write(&deep.join("file.txt"), b"x");

    let result = prepare_transfer_with_id(&[dir.path().join("top")], ID);
    assert!(matches!(result, Err(BeamError::Selection(_))));
}

#[test]
fn reports_missing_paths_as_storage_errors() {
    let dir = TempDir::new().expect("temp dir");
    let missing = dir.path().join("gone.txt");

    let err = prepare_transfer_with_id(&[missing], ID).expect_err("missing file");
    assert_eq!(err.category(), ErrorCategory::Storage);
    assert!(err.to_string().contains("gone.txt"));
}

#[test]
fn detects_source_files_that_changed_after_the_offer() {
    let dir = TempDir::new().expect("temp dir");
    let file = dir.path().join("draft.txt");
    write(&file, b"first");
    let prepared = prepare_transfer(std::slice::from_ref(&file)).expect("prepare");
    let source = prepared.source(0).expect("source");
    source.ensure_unchanged().expect("unchanged");

    write(&file, b"first, then more");
    assert!(matches!(
        source.ensure_unchanged(),
        Err(BeamError::SourceChanged { .. })
    ));

    fs::remove_file(&file).expect("remove");
    let err = source.ensure_unchanged().expect_err("missing");
    assert_eq!(err.category(), ErrorCategory::Storage);
}
