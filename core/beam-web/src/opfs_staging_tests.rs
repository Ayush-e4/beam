//! The `data` layout and folder names; staging itself runs in a browser, see
//! `tests/opfs_staging.rs`.

use std::sync::Arc;

use beam_core::{ReadAt, ReadableFile, prepare_readable};

use super::*;

#[derive(Debug)]
struct Sized(u64);

impl ReadAt for Sized {
    fn size(&self) -> io::Result<u64> {
        Ok(self.0)
    }

    fn modified(&self) -> io::Result<Option<std::time::SystemTime>> {
        Ok(None)
    }

    fn read_exact_at(&self, buf: &mut [u8], _offset: u64) -> io::Result<()> {
        buf.fill(0);
        Ok(())
    }
}

fn manifest() -> Manifest {
    let files = [("a", 10), ("b", 0), ("c", 7)].map(|(name, size)| ReadableFile {
        name: name.to_owned(),
        reader: Arc::new(Sized(size)),
    });
    prepare_readable(files.into()).unwrap().manifest().clone()
}

fn spans(items: &[StagedItem]) -> Vec<(&str, u64, u64)> {
    items
        .iter()
        .map(|i| (i.path.as_str(), i.offset, i.size))
        .collect()
}

#[test]
fn items_sit_back_to_back_in_manifest_order() {
    let items = layout(&manifest(), &[]).unwrap();
    assert_eq!(spans(&items), [("a", 0, 10), ("b", 10, 0), ("c", 10, 7)]);
}

#[test]
fn only_selected_items_take_space() {
    let manifest = manifest();
    let c = manifest.items()[2].item_id;
    assert_eq!(spans(&layout(&manifest, &[c]).unwrap()), [("c", 0, 7)]);
}

#[test]
fn staging_belongs_to_one_sender_and_one_choice_of_items() {
    let manifest = manifest();
    let all = layout(&manifest, &[]).unwrap();
    let some = layout(&manifest, &[manifest.items()[0].item_id]).unwrap();
    let name = folder_name(&manifest, &all, &[1; 32]);
    assert_eq!(name, folder_name(&manifest, &all, &[1; 32]));
    assert_ne!(name, folder_name(&manifest, &all, &[2; 32]));
    assert_ne!(name, folder_name(&manifest, &some, &[1; 32]));
    let transfer = manifest.transfer_id().0;
    assert!(name.starts_with(&format!("{:02x}", transfer[0])));
}
