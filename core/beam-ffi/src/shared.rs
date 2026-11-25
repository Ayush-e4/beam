//! Files the app shares by file descriptor. Android gives gallery, share-sheet and picker items
//! as descriptors that can't be reopened by path, so the engine reads from its own copy of
//! each descriptor.

use std::fs::File;

use beam_core::OpenedFile;

use crate::BeamError;

/// A file the app has open, to send under `name`.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct SharedFile {
    /// The name to send it under, such as the provider's display name.
    pub name: String,
    /// An open, readable file descriptor of a regular file. The engine takes its own copy
    /// during the call; the app keeps this one, holds it open until the call returns and then
    /// closes it.
    pub fd: i32,
}

/// The engine's own open copy of each file in `files`.
pub(crate) fn duplicate(files: &[SharedFile]) -> Result<Vec<OpenedFile>, BeamError> {
    files
        .iter()
        .map(|shared| {
            Ok(OpenedFile {
                name: shared.name.clone(),
                file: duplicate_fd(shared.fd).map_err(|detail| BeamError::Storage {
                    detail: format!("can't read {}: {detail}", shared.name),
                })?,
            })
        })
        .collect()
}

/// A new descriptor for the same open file (`dup`), owned by the engine.
#[cfg(unix)]
#[allow(unsafe_code)] // The only unsafe code in Beam; see the SAFETY note.
fn duplicate_fd(fd: i32) -> Result<File, String> {
    use std::os::fd::BorrowedFd;

    if fd < 0 {
        return Err(format!("{fd} is not a file descriptor"));
    }
    // SAFETY: `borrow_raw` requires `fd` to stay open for as long as the borrow lives. The
    // borrow lives only for the `dup` below, and `SharedFile` requires the app to hold `fd`
    // open until this call returns. Nothing here closes or takes ownership of `fd`. If `fd`
    // isn't open, `dup` fails with EBADF and is reported as an error.
    let borrowed = unsafe { BorrowedFd::borrow_raw(fd) };
    let owned = borrowed
        .try_clone_to_owned()
        .map_err(|error| error.to_string())?;
    Ok(File::from(owned))
}

#[cfg(not(unix))]
fn duplicate_fd(_fd: i32) -> Result<File, String> {
    Err("file descriptors are only supported on Android and other Unix systems".into())
}
