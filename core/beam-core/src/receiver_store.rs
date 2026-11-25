//! On-disk staging for an incoming transfer.
//!
//! Files are received into a staging folder ([`crate::staging`]) as `<item>.part`, next
//! to the resume journal and one `<item>.root` file per item holding its root hash. Staging
//! lives inside the destination so publishing a finished file is a same-file-system rename,
//! which is atomic: a file appears under its real name only once it is complete and verified.

use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::time::{Duration, UNIX_EPOCH};

use crate::integrity::Hash;
use crate::manifest::ManifestItem;
use crate::{BeamError, Result};

/// Folder inside the destination that holds transfers in progress.
pub const STAGING_DIR: &str = ".beam-partial";

const MAX_NAME_ATTEMPTS: u32 = 10_000;

/// Staging files for one transfer.
#[derive(Debug)]
pub(crate) struct Store {
    destination: PathBuf,
    staging: PathBuf,
    open: HashMap<u32, File>,
}

impl Store {
    /// Uses `staging` (from [`crate::staging::claim`]) for the transfer's files.
    pub(crate) fn open(destination: &Path, staging: PathBuf) -> Self {
        Self {
            destination: destination.to_path_buf(),
            staging,
            open: HashMap::new(),
        }
    }

    pub(crate) fn journal_path(&self) -> PathBuf {
        self.staging.join(crate::staging::JOURNAL_FILE)
    }

    fn part_path(&self, item_id: u32) -> PathBuf {
        self.staging.join(format!("{item_id}.part"))
    }

    fn root_path(&self, item_id: u32) -> PathBuf {
        self.staging.join(format!("{item_id}.root"))
    }

    fn file(&mut self, item_id: u32) -> Result<&mut File> {
        if !self.open.contains_key(&item_id) {
            let file = OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(false)
                .open(self.part_path(item_id))
                .map_err(BeamError::storage)?;
            self.open.insert(item_id, file);
        }
        self.open
            .get_mut(&item_id)
            .ok_or_else(|| BeamError::storage(std::io::Error::other("staging file vanished")))
    }

    /// Writes `data` at `offset` in the item's staging file.
    pub(crate) fn write_at(&mut self, item_id: u32, offset: u64, data: &[u8]) -> Result<()> {
        let file = self.file(item_id)?;
        file.seek(SeekFrom::Start(offset))
            .map_err(BeamError::storage)?;
        file.write_all(data).map_err(BeamError::storage)
    }

    /// Replaces the item's staging file with `data` (used for packed small files).
    pub(crate) fn write_whole(&mut self, item_id: u32, data: &[u8]) -> Result<()> {
        let file = self.file(item_id)?;
        file.set_len(0).map_err(BeamError::storage)?;
        file.seek(SeekFrom::Start(0)).map_err(BeamError::storage)?;
        file.write_all(data).map_err(BeamError::storage)
    }

    /// Flushes the item's written data to disk.
    pub(crate) fn sync(&mut self, item_id: u32) -> Result<()> {
        match self.open.get(&item_id) {
            Some(file) => file.sync_data().map_err(BeamError::storage),
            None => Ok(()),
        }
    }

    /// Releases the file handle of an item that needs no more writes.
    pub(crate) fn close(&mut self, item_id: u32) {
        self.open.remove(&item_id);
    }

    pub(crate) fn read_range(&mut self, item_id: u32, range: Range<u64>) -> Result<Vec<u8>> {
        let len = usize::try_from(range.end - range.start)
            .map_err(|_| BeamError::Malformed("chunk does not fit in memory".into()))?;
        let file = self.file(item_id)?;
        file.seek(SeekFrom::Start(range.start))
            .map_err(BeamError::storage)?;
        let mut data = vec![0u8; len];
        file.read_exact(&mut data).map_err(BeamError::storage)?;
        Ok(data)
    }

    /// Durably stores an item's root hash (write, sync, then rename into place).
    pub(crate) fn save_root(&self, item_id: u32, root: &Hash) -> Result<()> {
        let path = self.root_path(item_id);
        let tmp = path.with_extension("root.tmp");
        let mut file = File::create(&tmp).map_err(BeamError::storage)?;
        file.write_all(root).map_err(BeamError::storage)?;
        file.sync_all().map_err(BeamError::storage)?;
        fs::rename(&tmp, &path).map_err(BeamError::storage)
    }

    pub(crate) fn load_root(&self, item_id: u32) -> Result<Hash> {
        let bytes = fs::read(self.root_path(item_id)).map_err(BeamError::storage)?;
        bytes
            .try_into()
            .map_err(|_| BeamError::Malformed(format!("root hash of item {item_id} is damaged")))
    }

    /// Moves a finished item to its final path, never overwriting an existing file: a clash
    /// gets a numbered name such as `photo (1).jpg`. Returns where the file ended up.
    pub(crate) fn publish(&mut self, item: &ManifestItem) -> Result<PathBuf> {
        self.close(item.item_id);
        let part = self.part_path(item.item_id);
        if item.size == 0 {
            File::create(&part).map_err(BeamError::storage)?;
        }
        if let Some(ms) = item.modified_at_ms.and_then(|ms| u64::try_from(ms).ok()) {
            let file = File::options()
                .write(true)
                .open(&part)
                .map_err(BeamError::storage)?;
            // Best effort: some file systems refuse timestamps; the content is what matters.
            let _ = file.set_modified(UNIX_EPOCH + Duration::from_millis(ms));
        }
        let wanted = item.path.resolve_in(&self.destination);
        if let Some(parent) = wanted.parent() {
            fs::create_dir_all(parent).map_err(BeamError::storage)?;
        }
        let target = free_name(&wanted)?;
        fs::rename(&part, &target).map_err(BeamError::storage)?;
        Ok(target)
    }

    /// Removes the staging folder once the transfer is published.
    pub(crate) fn remove_staging(self) -> Result<()> {
        drop(self.open);
        fs::remove_dir_all(&self.staging).map_err(BeamError::storage)?;
        // Leave the shared parent if another transfer is still using it.
        let _ = fs::remove_dir(self.destination.join(STAGING_DIR));
        Ok(())
    }
}

/// `wanted` if nothing is there, otherwise the first free `name (n).ext`.
fn free_name(wanted: &Path) -> Result<PathBuf> {
    if !wanted.exists() {
        return Ok(wanted.to_path_buf());
    }
    let stem = wanted
        .file_stem()
        .map_or_else(String::new, |s| s.to_string_lossy().into_owned());
    let ext = wanted
        .extension()
        .map_or_else(String::new, |e| format!(".{}", e.to_string_lossy()));
    (1..=MAX_NAME_ATTEMPTS)
        .map(|n| wanted.with_file_name(format!("{stem} ({n}){ext}")))
        .find(|candidate| !candidate.exists())
        .ok_or_else(|| BeamError::storage(std::io::Error::other("no free file name")))
}
