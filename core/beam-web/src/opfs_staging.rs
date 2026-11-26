//! A transfer received into the browser's origin-private file system (OPFS).
//!
//! OPFS reads and writes are synchronous in a worker, but opening, renaming and removing files
//! are not, and the receiver runs synchronously. So everything is opened up front, three files
//! per transfer in `beam-receiving/<transfer>-<owner>/`:
//!
//! - `data`: every selected item back to back, at the offsets [`layout`] gives;
//! - `roots`: each item's root hash, in a fixed slot;
//! - `journal`: the resume journal.
//!
//! Publishing an item leaves it where it is. Once the transfer finishes, the web app saves each
//! item from its slice of `data`, then calls [`remove_staged`].

use std::fmt::Write as _;
use std::io;
use std::ops::Range;
use std::path::PathBuf;

use beam_core::integrity::chunk_hash;
use beam_core::{BeamError, Hash, Manifest, ManifestItem, Result, StorageFile, TransferStaging};
use web_sys::{FileSystemDirectoryHandle, FileSystemRemoveOptions};

use crate::opfs_file::{OpfsFile, folder, resolve};

/// The OPFS folder holding every transfer being received.
const RECEIVING: &str = "beam-receiving";
/// A root slot: a byte saying it is set, then the hash.
const ROOT_LEN: usize = 1 + 32;
const ROOT_SLOT: u64 = ROOT_LEN as u64;

/// Where one item sits in the `data` file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StagedItem {
    /// The item's ID in the manifest.
    pub item_id: u32,
    /// Its relative path, as the sender named it.
    pub path: String,
    /// Where its bytes start in `data`.
    pub offset: u64,
    /// How many bytes it has.
    pub size: u64,
}

/// The selected items of `manifest` (all of them if `selection` is empty) and where each sits
/// in the `data` file, in manifest order.
///
/// # Errors
///
/// [`BeamError::Malformed`] if the items add up to more than a file can hold.
pub fn layout(manifest: &Manifest, selection: &[u32]) -> Result<Vec<StagedItem>> {
    let mut offset = 0u64;
    let mut items = Vec::new();
    for item in manifest.items() {
        if !selection.is_empty() && !selection.contains(&item.item_id) {
            continue;
        }
        items.push(StagedItem {
            item_id: item.item_id,
            path: item.path.to_string(),
            offset,
            size: item.size,
        });
        offset = offset
            .checked_add(item.size)
            .ok_or_else(|| BeamError::Malformed("transfer is too large".into()))?;
    }
    Ok(items)
}

/// The folder name for `manifest`'s transfer from `sender` with these items. Two senders
/// reusing a transfer ID never share staging, and nor does another choice of items, whose
/// offsets in `data` differ.
fn folder_name(manifest: &Manifest, items: &[StagedItem], sender: &[u8]) -> String {
    let mut owner = sender.to_vec();
    for item in items {
        owner.extend_from_slice(&item.item_id.to_le_bytes());
    }
    let hex = |bytes: &[u8]| {
        bytes.iter().fold(String::new(), |mut out, b| {
            let _ = write!(out, "{b:02x}"); // writing to a String can't fail
            out
        })
    };
    let digest = chunk_hash(&owner);
    format!("{}-{}", hex(&manifest.transfer_id().0), hex(&digest[..16]))
}

/// One transfer's staging in OPFS. Dropping it closes its files.
#[derive(Debug)]
pub struct OpfsStaging {
    items: Vec<StagedItem>,
    data: OpfsFile,
    roots: OpfsFile,
    journal: Option<OpfsFile>,
}

impl OpfsStaging {
    /// Opens (or creates) the staging for `manifest`'s transfer from `sender` under the OPFS
    /// folder `root`, keeping whatever an earlier session left there.
    ///
    /// # Errors
    ///
    /// A storage error if the files can't be opened, including when another tab has this
    /// transfer open; [`BeamError::Malformed`] if the transfer is too large.
    pub async fn open(
        root: &FileSystemDirectoryHandle,
        manifest: &Manifest,
        selection: &[u32],
        sender: &[u8],
    ) -> Result<Self> {
        let items = layout(manifest, selection)?;
        let receiving = folder(root, RECEIVING).await.map_err(BeamError::storage)?;
        let dir = folder(&receiving, &folder_name(manifest, &items, sender))
            .await
            .map_err(BeamError::storage)?;
        let open = |name| OpfsFile::open(&dir, name);
        let data = open("data").await.map_err(BeamError::storage)?;
        let roots = open("roots").await.map_err(BeamError::storage)?;
        let journal = open("journal").await.map_err(BeamError::storage)?;
        Ok(Self {
            items,
            data,
            roots,
            journal: Some(journal),
        })
    }

    /// Where the item's bytes are, and its slot in `roots`.
    fn find(&self, item_id: u32) -> Result<(&StagedItem, u64)> {
        let slot = self.items.iter().position(|i| i.item_id == item_id);
        let slot = slot.ok_or_else(|| BeamError::Malformed("item isn't staged".into()))?;
        Ok((&self.items[slot], slot as u64 * ROOT_SLOT))
    }

    /// Where `len` bytes at `offset` in the item go in `data`, refusing to spill into the next.
    fn place(&self, item_id: u32, offset: u64, len: usize) -> Result<u64> {
        let (item, _) = self.find(item_id)?;
        let end = offset.checked_add(len as u64);
        if end.is_none_or(|end| end > item.size) {
            return Err(BeamError::Malformed("write runs past the item".into()));
        }
        Ok(item.offset + offset)
    }
}

impl TransferStaging for OpfsStaging {
    fn journal(&mut self) -> Result<Box<dyn StorageFile>> {
        let journal = self.journal.take();
        let journal =
            journal.ok_or_else(|| BeamError::storage(io::ErrorKind::ResourceBusy.into()))?;
        Ok(Box::new(journal))
    }

    fn write_at(&mut self, item_id: u32, offset: u64, data: &[u8]) -> Result<()> {
        let at = self.place(item_id, offset, data.len())?;
        self.data.write_all_at(data, at).map_err(BeamError::storage)
    }

    fn write_whole(&mut self, item_id: u32, data: &[u8]) -> Result<()> {
        if data.len() as u64 != self.find(item_id)?.0.size {
            return Err(BeamError::Malformed("file size doesn't match".into()));
        }
        self.write_at(item_id, 0, data)
    }

    fn sync(&mut self, _item_id: u32) -> Result<()> {
        self.data.sync().map_err(BeamError::storage)
    }

    fn close(&mut self, _item_id: u32) {} // one file holds every item

    fn read_range(&mut self, item_id: u32, range: Range<u64>) -> Result<Vec<u8>> {
        let len = usize::try_from(range.end.saturating_sub(range.start))
            .map_err(|_| BeamError::Malformed("range is too large".into()))?;
        let at = self.place(item_id, range.start, len)?;
        let mut buf = vec![0; len];
        self.data
            .read_exact_at(&mut buf, at)
            .map_err(BeamError::storage)?;
        Ok(buf)
    }

    fn save_root(&mut self, item_id: u32, root: &Hash) -> Result<()> {
        let (_, slot) = self.find(item_id)?;
        let mut record = [1; ROOT_LEN];
        record[1..].copy_from_slice(root);
        self.roots
            .write_all_at(&record, slot)
            .and_then(|()| self.roots.sync())
            .map_err(BeamError::storage)
    }

    fn load_root(&mut self, item_id: u32) -> Result<Hash> {
        let (_, slot) = self.find(item_id)?;
        let mut record = [0; ROOT_LEN];
        let read = self.roots.read_exact_at(&mut record, slot);
        if read.is_err() || record[0] != 1 {
            return Err(BeamError::storage(io::ErrorKind::NotFound.into()));
        }
        let mut root = [0; 32];
        root.copy_from_slice(&record[1..]);
        Ok(root)
    }

    fn publish(&mut self, item: &ManifestItem) -> Result<PathBuf> {
        self.find(item.item_id)?; // stays in `data` until the web app saves it
        Ok(PathBuf::from(item.path.to_string()))
    }

    fn remove(self: Box<Self>) -> Result<()> {
        Ok(()) // closes the files; the web app removes them once everything is saved
    }
}

/// Removes the staging [`OpfsStaging::open`] opened with the same arguments. Call it once every
/// item is saved, or to abandon the transfer; nothing may have it open.
///
/// # Errors
///
/// A storage error if it can't be removed, for example while it is still open.
pub async fn remove_staged(
    root: &FileSystemDirectoryHandle,
    manifest: &Manifest,
    selection: &[u32],
    sender: &[u8],
) -> Result<()> {
    let items = layout(manifest, selection)?;
    let receiving = folder(root, RECEIVING).await.map_err(BeamError::storage)?;
    let options = FileSystemRemoveOptions::new();
    options.set_recursive(true);
    let removing =
        receiving.remove_entry_with_options(&folder_name(manifest, &items, sender), &options);
    resolve::<wasm_bindgen::JsValue>(removing)
        .await
        .map_err(BeamError::storage)?;
    Ok(())
}

/// The `data` file [`OpfsStaging::open`] filled with the same arguments, to save each item
/// from its slice (see [`layout`]). Read it before [`remove_staged`]: a removed file can't be
/// read any more.
///
/// # Errors
///
/// A storage error if there is no such staging.
pub async fn staged_data(
    root: &FileSystemDirectoryHandle,
    manifest: &Manifest,
    selection: &[u32],
    sender: &[u8],
) -> Result<web_sys::File> {
    let items = layout(manifest, selection)?;
    let receiving = folder(root, RECEIVING).await.map_err(BeamError::storage)?;
    let dir = folder(&receiving, &folder_name(manifest, &items, sender))
        .await
        .map_err(BeamError::storage)?;
    let data: web_sys::FileSystemFileHandle = resolve(dir.get_file_handle("data"))
        .await
        .map_err(BeamError::storage)?;
    resolve(data.get_file()).await.map_err(BeamError::storage)
}

#[cfg(test)]
#[path = "opfs_staging_tests.rs"]
mod tests;
