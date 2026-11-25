//! The file behind one manifest item: where it is (or the handle it was opened as), and what
//! it looked like when it was offered, so a file that changed since is caught.

use std::fs::{self, File, Metadata};
use std::io;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::SystemTime;

use crate::sender_files::storage_error;
use crate::{BeamError, Result};

/// A file on disk behind one manifest item, with what it looked like when it was offered.
#[derive(Debug, Clone)]
pub struct SourceFile {
    /// Where the file is on this device; for an [`opened`](Self::opened) file, its name.
    pub path: PathBuf,
    /// Its size when the manifest was built.
    pub size: u64,
    /// Its modified time when the manifest was built, if the platform reports one.
    pub modified: Option<SystemTime>,
    /// The file, already open, when it has no path this process can open.
    pub opened: Option<Arc<File>>,
}

impl PartialEq for SourceFile {
    fn eq(&self, other: &Self) -> bool {
        let same_handle = match (&self.opened, &other.opened) {
            (Some(a), Some(b)) => Arc::ptr_eq(a, b),
            (None, None) => true,
            _ => false,
        };
        same_handle
            && self.path == other.path
            && self.size == other.size
            && self.modified == other.modified
    }
}

impl Eq for SourceFile {}

impl SourceFile {
    /// Opens the file for reading: a new handle to an [`opened`](Self::opened) file (read it by
    /// position; the handles share one offset), or the file at [`path`](Self::path).
    ///
    /// # Errors
    ///
    /// Returns a storage error if it can't be opened.
    pub fn open(&self) -> Result<File> {
        let file = match &self.opened {
            Some(file) => file.try_clone(),
            None => File::open(&self.path),
        };
        file.map_err(|err| storage_error(&self.path, &err))
    }

    /// Reads the whole file (a small one), checking it still has the offered size.
    ///
    /// # Errors
    ///
    /// [`BeamError::SourceChanged`] if it changed, a storage error if it can't be read.
    pub fn read_whole(&self) -> Result<Vec<u8>> {
        self.ensure_unchanged()?;
        let len = usize::try_from(self.size).map_err(|_| self.changed())?;
        let mut data = vec![0; len];
        match read_exact_at(&mut self.open()?, &mut data, 0) {
            Ok(()) => Ok(data),
            Err(err) if err.kind() == io::ErrorKind::UnexpectedEof => Err(self.changed()),
            Err(err) => Err(storage_error(&self.path, &err)),
        }
    }

    fn metadata(&self) -> io::Result<Metadata> {
        match &self.opened {
            Some(file) => file.metadata(),
            None => fs::metadata(&self.path),
        }
    }

    /// Checks that the file still has the size and modified time it had when it was offered.
    ///
    /// # Errors
    ///
    /// Returns [`BeamError::SourceChanged`] if it changed, or a storage error if it can no
    /// longer be read.
    pub fn ensure_unchanged(&self) -> Result<()> {
        let meta = self
            .metadata()
            .map_err(|err| storage_error(&self.path, &err))?;
        if meta.len() != self.size || meta.modified().ok() != self.modified {
            return Err(self.changed());
        }
        Ok(())
    }

    /// The error for this file having changed since the offer.
    #[must_use]
    pub fn changed(&self) -> BeamError {
        BeamError::SourceChanged {
            path: self.path.display().to_string(),
        }
    }
}

/// Reads exactly `buf.len()` bytes at `offset`. On Unix this doesn't touch the file's offset,
/// which an already-open file shares with every handle cloned from it.
pub(crate) fn read_exact_at(file: &mut File, buf: &mut [u8], offset: u64) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::FileExt;
        file.read_exact_at(buf, offset)
    }
    #[cfg(not(unix))]
    {
        use std::io::{Read, Seek, SeekFrom};
        file.seek(SeekFrom::Start(offset))?;
        file.read_exact(buf)
    }
}
