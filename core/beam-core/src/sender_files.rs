//! Building a transfer manifest from the files and folders a user picked to send.
//!
//! - Each picked file or folder lands at the top of the receiver's destination under its own
//!   name; folder contents keep their relative layout. Folders are listed in name order.
//! - Names are sanitized as the receiver would sanitize them, and names that would collide get a
//!   ` (2)`, ` (3)`, … suffix (see [`NameAllocator`]), so the manifest always validates.
//! - A picked path that is a symbolic link is followed (the user chose it), but links found
//!   inside folders are skipped: following them could loop or send files outside the selection.
//!   Sockets, pipes and devices are skipped too.
//! - Empty folders are not sent, since a manifest only lists files.
//!
//! - Files can also be handed over already open ([`crate::sender_opened`]), as Android does
//!   for gallery, share-sheet and picker items that have no readable path.
//!
//! Everything here does blocking file I/O; async callers should run it on a blocking thread.

use std::collections::HashMap;
use std::fs::{self, Metadata};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::manifest::{MAX_ITEMS, Manifest, ManifestItem, TransferId};
use crate::read_at::ReadAt;
use crate::safe_path::{MAX_DEPTH, SafePath};
use crate::sender_names::NameAllocator;
use crate::{BeamError, Result, mime};

pub use crate::sender_source::SourceFile;

/// Chunk size used for every file the sender offers (4 MiB).
pub const DEFAULT_CHUNK_SIZE: u32 = 4 * 1024 * 1024;

/// A manifest ready to offer, plus where each of its items lives on disk.
#[derive(Debug, Clone)]
pub struct PreparedTransfer {
    manifest: Manifest,
    sources: HashMap<u32, SourceFile>,
}

impl PreparedTransfer {
    /// The manifest to send in the `OFFER`.
    #[must_use]
    pub fn manifest(&self) -> &Manifest {
        &self.manifest
    }

    /// The file behind a manifest item.
    #[must_use]
    pub fn source(&self, item_id: u32) -> Option<&SourceFile> {
        self.sources.get(&item_id)
    }
}

/// Walks the picked files and folders and builds a manifest with a new random transfer ID.
///
/// # Errors
///
/// Returns a storage error if a picked path or folder cannot be read, and
/// [`BeamError::Selection`] if there is nothing to send, too many files, or folders nested
/// deeper than a receiver accepts.
pub fn prepare_transfer(selection: &[PathBuf]) -> Result<PreparedTransfer> {
    prepare_transfer_with_id(selection, TransferId::random()?)
}

/// Like [`prepare_transfer`], with a given transfer ID.
///
/// # Errors
///
/// As for [`prepare_transfer`].
pub fn prepare_transfer_with_id(
    selection: &[PathBuf],
    transfer_id: TransferId,
) -> Result<PreparedTransfer> {
    let mut builder = Builder::default();
    let mut names = NameAllocator::default();
    for path in selection {
        let meta = fs::metadata(path).map_err(|err| storage_error(path, &err))?;
        let mut relative = vec![names.allocate(&display_name(path))];
        builder.add(path, &meta, &mut relative)?;
    }
    builder.finish(transfer_id)
}

#[derive(Default)]
pub(crate) struct Builder {
    items: Vec<ManifestItem>,
    sources: HashMap<u32, SourceFile>,
    total: u64,
}

impl Builder {
    pub(crate) fn finish(self, transfer_id: TransferId) -> Result<PreparedTransfer> {
        if self.items.is_empty() {
            return Err(selection_error("there are no files to send"));
        }
        let manifest = Manifest::new(transfer_id, self.items, self.total)?;
        Ok(PreparedTransfer {
            manifest,
            sources: self.sources,
        })
    }

    /// Adds a file or folder whose receiver-side path is `relative`.
    fn add(&mut self, path: &Path, meta: &Metadata, relative: &mut Vec<String>) -> Result<()> {
        if meta.is_dir() {
            self.add_folder(path, relative)
        } else if meta.is_file() {
            self.add_file(path, meta.len(), meta.modified().ok(), relative, None)
        } else {
            Ok(()) // Symbolic links inside folders, sockets, pipes and devices.
        }
    }

    fn add_folder(&mut self, folder: &Path, relative: &mut Vec<String>) -> Result<()> {
        if relative.len() >= MAX_DEPTH {
            return Err(selection_error(&format!(
                "folders are nested more than {MAX_DEPTH} deep in {}",
                folder.display()
            )));
        }
        let mut entries = fs::read_dir(folder)
            .and_then(Iterator::collect::<io::Result<Vec<_>>>)
            .map_err(|err| storage_error(folder, &err))?;
        entries.sort_by_key(fs::DirEntry::file_name);

        let mut names = NameAllocator::default();
        for entry in entries {
            let path = entry.path();
            // `DirEntry::metadata` does not follow symbolic links, so links are skipped by `add`.
            let meta = entry.metadata().map_err(|err| storage_error(&path, &err))?;
            if !meta.is_dir() && !meta.is_file() {
                continue;
            }
            relative.push(names.allocate(&entry.file_name().to_string_lossy()));
            let added = self.add(&path, &meta, relative);
            relative.pop();
            added?;
        }
        Ok(())
    }

    /// Adds one file of `size` bytes whose receiver-side path is `relative`; `opened` if it is
    /// already open.
    pub(crate) fn add_file(
        &mut self,
        path: &Path,
        size: u64,
        modified: Option<SystemTime>,
        relative: &[String],
        opened: Option<Arc<dyn ReadAt>>,
    ) -> Result<()> {
        if self.items.len() >= MAX_ITEMS {
            return Err(selection_error(&format!(
                "more than {MAX_ITEMS} files are selected"
            )));
        }
        let joined = relative.join("/");
        let safe_path = SafePath::parse(&joined)
            .map_err(|_| selection_error(&format!("the path is too long: {joined}")))?;
        let item_id = u32::try_from(self.items.len())
            .map_err(|_| selection_error("too many files are selected"))?;

        self.total = self
            .total
            .checked_add(size)
            .ok_or_else(|| selection_error("the files are too large in total"))?;
        self.items.push(ManifestItem {
            item_id,
            mime: mime::guess(safe_path.file_name()).to_owned(),
            path: safe_path,
            size,
            modified_at_ms: modified.and_then(unix_millis),
            chunk_size: DEFAULT_CHUNK_SIZE,
        });
        self.sources.insert(
            item_id,
            SourceFile {
                path: path.to_path_buf(),
                size,
                modified,
                opened,
            },
        );
        Ok(())
    }
}

/// The name a picked path is sent under: its last component, or the whole path for roots.
fn display_name(path: &Path) -> String {
    path.file_name().map_or_else(
        || path.display().to_string(),
        |name| name.to_string_lossy().into_owned(),
    )
}

fn unix_millis(time: SystemTime) -> Option<i64> {
    let since_epoch = time.duration_since(UNIX_EPOCH).ok()?;
    i64::try_from(since_epoch.as_millis()).ok()
}

pub(crate) fn selection_error(reason: &str) -> BeamError {
    BeamError::Selection(reason.to_owned())
}

/// Wraps an I/O error as a storage error that names the file it happened on.
pub(crate) fn storage_error(path: &Path, err: &io::Error) -> BeamError {
    BeamError::storage(io::Error::new(
        err.kind(),
        format!("{}: {err}", path.display()),
    ))
}

#[cfg(test)]
#[path = "sender_files_tests.rs"]
mod tests;
