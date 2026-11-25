//! Sending files that are handed over already open, with only a name. Android gives apps
//! gallery, share-sheet and picker items this way: there is no path the app may open, only a
//! file descriptor. Each file lands at the top of the receiver's destination under its name.

use std::fs::File;
use std::path::PathBuf;
use std::sync::Arc;

use crate::Result;
use crate::manifest::TransferId;
use crate::sender_files::{Builder, PreparedTransfer, selection_error, storage_error};
use crate::sender_names::NameAllocator;

/// A file to send, already open for reading.
#[derive(Debug)]
pub struct OpenedFile {
    /// The name to send it under, such as `IMG_0042.jpg`. Sanitized like any other name.
    pub name: String,
    /// The file, open for reading. It must support reading at any position.
    pub file: File,
}

/// Prepares a transfer of `files`, in order, with a fresh transfer ID.
///
/// # Errors
///
/// [`crate::BeamError::Selection`] if there is nothing to send, one isn't a regular file or
/// there are too many; a storage error if one can't be inspected.
pub fn prepare_opened(files: Vec<OpenedFile>) -> Result<PreparedTransfer> {
    prepare_opened_with_id(files, TransferId::random()?)
}

/// Like [`prepare_opened`], with a given transfer ID.
///
/// # Errors
///
/// As for [`prepare_opened`].
pub fn prepare_opened_with_id(
    files: Vec<OpenedFile>,
    transfer_id: TransferId,
) -> Result<PreparedTransfer> {
    let mut builder = Builder::default();
    let mut names = NameAllocator::default();
    for OpenedFile { name, file } in files {
        let label = PathBuf::from(&name);
        let meta = file.metadata().map_err(|err| storage_error(&label, &err))?;
        if !meta.is_file() {
            return Err(selection_error(&format!("{name} is not a file")));
        }
        let relative = [names.allocate(&name)];
        builder.add_file(&label, &meta, &relative, Some(Arc::new(file)))?;
    }
    builder.finish(transfer_id)
}

#[cfg(test)]
#[path = "sender_opened_tests.rs"]
mod tests;
