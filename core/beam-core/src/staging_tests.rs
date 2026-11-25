//! Staging folders: one per transfer and sender, unusable ones skipped, stale ones removed.

use std::fs;
use std::time::{Duration, SystemTime};

use tempfile::TempDir;

use super::*;
use crate::Receiver;
use crate::manifest::{MIN_CHUNK_SIZE, Manifest, ManifestItem};
use crate::safe_path::SafePath;

const ID: TransferId = TransferId([7; 16]);
const LAPTOP: &[u8] = &[1; 32];
const TABLET: &[u8] = &[2; 32];
const WEEK: Duration = Duration::from_hours(24 * 7);

fn manifest() -> Manifest {
    manifest_of(ID, "clip.bin")
}

fn manifest_of(id: TransferId, name: &str) -> Manifest {
    let item = ManifestItem {
        item_id: 1,
        path: SafePath::parse(name).unwrap(),
        size: u64::from(MIN_CHUNK_SIZE) * 2,
        mime: "application/octet-stream".into(),
        modified_at_ms: None,
        chunk_size: MIN_CHUNK_SIZE,
    };
    let total = item.size;
    Manifest::new(id, vec![item], total).unwrap()
}

fn open(dest: &TempDir, sender: &[u8]) -> Receiver {
    Receiver::open_from(manifest(), dest.path(), &[], sender).unwrap()
}

#[test]
fn the_same_device_continues_and_another_never_mixes_in() {
    let dest = TempDir::new().unwrap();
    assert!(!continues(dest.path(), &manifest(), LAPTOP));
    drop(open(&dest, LAPTOP));
    assert!(continues(dest.path(), &manifest(), LAPTOP));
    assert!(!continues(dest.path(), &manifest(), TABLET));
    let other_id = manifest_of(TransferId([8; 16]), "clip.bin");
    assert!(!continues(dest.path(), &other_id, LAPTOP));
    assert!(
        !continues(dest.path(), &manifest(), &[]),
        "an unknown sender never continues"
    );

    assert!(!open(&dest, TABLET).resumed(), "the tablet starts its own");
    assert!(open(&dest, LAPTOP).resumed());
    assert!(open(&dest, TABLET).resumed());
}

#[test]
fn the_same_id_with_other_files_is_not_a_continuation() {
    let dest = TempDir::new().unwrap();
    drop(open(&dest, LAPTOP));
    let swapped = manifest_of(ID, "invoice.pdf.apk");
    assert!(
        !continues(dest.path(), &swapped, LAPTOP),
        "the person accepted clip.bin, not this"
    );
}

#[cfg(unix)]
#[test]
fn a_folder_that_cant_be_opened_is_skipped() {
    use std::os::unix::fs::PermissionsExt;

    let dest = TempDir::new().unwrap();
    drop(open(&dest, LAPTOP));
    // Like files an earlier install of the app left: still there, but not ours to open.
    let left = slot(dest.path(), ID, 0).join(SENDER_FILE);
    fs::set_permissions(&left, fs::Permissions::from_mode(0o000)).unwrap();

    let receiver = open(&dest, LAPTOP);
    assert!(!receiver.resumed());
    assert!(slot(dest.path(), ID, 1).join(JOURNAL_FILE).is_file());
    assert!(continues(dest.path(), &manifest(), LAPTOP));
    fs::set_permissions(&left, fs::Permissions::from_mode(0o600)).unwrap();
}

#[test]
fn only_transfers_without_progress_for_a_while_are_removed() {
    let dest = TempDir::new().unwrap();
    assert_eq!(
        remove_stale(dest.path(), WEEK, SystemTime::now()).unwrap(),
        Cleanup::default(),
        "nothing staged yet"
    );
    drop(open(&dest, LAPTOP));
    drop(open(&dest, TABLET));

    let tomorrow = SystemTime::now() + Duration::from_hours(24);
    assert_eq!(
        remove_stale(dest.path(), WEEK, tomorrow).unwrap(),
        Cleanup::default()
    );
    assert!(continues(dest.path(), &manifest(), LAPTOP));

    let next_week = SystemTime::now() + WEEK + Duration::from_secs(60);
    let cleanup = remove_stale(dest.path(), WEEK, next_week).unwrap();
    assert_eq!(
        cleanup,
        Cleanup {
            removed: 2,
            failed: 0
        }
    );
    assert!(!continues(dest.path(), &manifest(), LAPTOP));
    assert!(!dest.path().join(STAGING_DIR).exists());
}
