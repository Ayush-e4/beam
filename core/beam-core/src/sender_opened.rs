//! Sending files that are handed over already open, with only a name. Android gives apps
//! gallery, share-sheet and picker items this way (a file descriptor, no path the app may
//! open), and browsers give picked files this way. Each file lands at the top of the
//! receiver's destination under its name.

use std::fs::File;
use std::path::PathBuf;
use std::sync::Arc;

use crate::Result;
use crate::manifest::TransferId;
use crate::read_at::ReadAt;
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

/// A file to send that the engine reads by position, with only a name: a file picked in a
/// browser, say. Unlike [`OpenedFile`] it isn't checked for being a regular file, as there is
/// nothing else it could be.
#[derive(Debug)]
pub struct ReadableFile {
    /// The name to send it under. Sanitized like any other name.
    pub name: String,
    /// How to read it.
    pub reader: Arc<dyn ReadAt>,
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
    let mut readable = Vec::with_capacity(files.len());
    for OpenedFile { name, file } in files {
        let meta = file
            .metadata()
            .map_err(|err| storage_error(&PathBuf::from(&name), &err))?;
        if !meta.is_file() {
            return Err(selection_error(&format!("{name} is not a file")));
        }
        readable.push(ReadableFile {
            name,
            reader: Arc::new(file),
        });
    }
    prepare_readable_with_id(readable, transfer_id)
}

/// Prepares a transfer of `files`, in order, with a fresh transfer ID.
///
/// # Errors
///
/// [`crate::BeamError::Selection`] if there is nothing to send or there are too many; a storage
/// error if one can't be inspected.
pub fn prepare_readable(files: Vec<ReadableFile>) -> Result<PreparedTransfer> {
    prepare_readable_with_id(files, TransferId::random()?)
}

/// Like [`prepare_readable`], with a given transfer ID.
///
/// # Errors
///
/// As for [`prepare_readable`].
pub fn prepare_readable_with_id(
    files: Vec<ReadableFile>,
    transfer_id: TransferId,
) -> Result<PreparedTransfer> {
    let mut builder = Builder::default();
    let mut names = NameAllocator::default();
    for ReadableFile { name, reader } in files {
        let label = PathBuf::from(&name);
        let inspect = |err| storage_error(&label, &err);
        let size = reader.size().map_err(inspect)?;
        let modified = reader.modified().map_err(inspect)?;
        let relative = [names.allocate(&name)];
        builder.add_file(&label, size, modified, &relative, Some(reader))?;
    }
    builder.finish(transfer_id)
}

#[cfg(test)]
#[path = "sender_opened_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "sender_readable_tests.rs"]
mod readable_tests;
