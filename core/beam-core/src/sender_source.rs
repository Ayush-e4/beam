//! The file behind one manifest item: where it is (or the handle it was opened as), and what
//! it looked like when it was offered, so a file that changed since is caught.

use std::fs::{self, File};
use std::io;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::SystemTime;

use crate::read_at::ReadAt;
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
    /// The file, already open, when it has no path this process can open (an Android
    /// content URI, a file picked in a browser).
    pub opened: Option<Arc<dyn ReadAt>>,
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
    /// Opens the file for reading by position: the [`opened`](Self::opened) file itself, or
    /// the file at [`path`](Self::path).
    ///
    /// # Errors
    ///
    /// Returns a storage error if it can't be opened.
    pub fn open(&self) -> Result<Arc<dyn ReadAt>> {
        match &self.opened {
            Some(file) => Ok(Arc::clone(file)),
            None => File::open(&self.path)
                .map(|file| Arc::new(file) as Arc<dyn ReadAt>)
                .map_err(|err| storage_error(&self.path, &err)),
        }
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
        match self.open()?.read_exact_at(&mut data, 0) {
            Ok(()) => Ok(data),
            Err(err) if err.kind() == io::ErrorKind::UnexpectedEof => Err(self.changed()),
            Err(err) => Err(storage_error(&self.path, &err)),
        }
    }

    /// Its size and modified time now.
    fn current(&self) -> io::Result<(u64, Option<SystemTime>)> {
        match &self.opened {
            Some(file) => Ok((file.size()?, file.modified()?)),
            None => fs::metadata(&self.path).map(|meta| (meta.len(), meta.modified().ok())),
        }
    }

    /// Checks that the file still has the size and modified time it had when it was offered.
    ///
    /// # Errors
    ///
    /// Returns [`BeamError::SourceChanged`] if it changed, or a storage error if it can no
    /// longer be read.
    pub fn ensure_unchanged(&self) -> Result<()> {
        let (size, modified) = self
            .current()
            .map_err(|err| storage_error(&self.path, &err))?;
        if size != self.size || modified != self.modified {
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
