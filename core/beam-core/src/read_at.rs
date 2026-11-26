//! Files the engine reads by position, whatever holds them: an open file on the phone or the
//! Mac, or a file picked in a browser (read with `FileReaderSync` in a worker).

use std::fmt;
use std::fs::File;
use std::io;
use std::time::SystemTime;

/// What a value needs to cross threads in the native engine. Browsers run the engine on one
/// thread, and their objects can't cross threads at all, so there it asks for nothing.
#[cfg(not(target_family = "wasm"))]
pub trait MaybeSend: Send + Sync {}
#[cfg(not(target_family = "wasm"))]
impl<T: Send + Sync + ?Sized> MaybeSend for T {}

/// What a value needs to cross threads in the native engine. Browsers run the engine on one
/// thread, and their objects can't cross threads at all, so there it asks for nothing.
#[cfg(target_family = "wasm")]
pub trait MaybeSend {}
#[cfg(target_family = "wasm")]
impl<T: ?Sized> MaybeSend for T {}

/// A file the engine can read at any position.
pub trait ReadAt: MaybeSend + fmt::Debug {
    /// Its size now, in bytes.
    ///
    /// # Errors
    ///
    /// If the file can no longer be inspected.
    fn size(&self) -> io::Result<u64>;

    /// When it was last modified, if the platform knows.
    ///
    /// # Errors
    ///
    /// If the file can no longer be inspected.
    fn modified(&self) -> io::Result<Option<SystemTime>>;

    /// Reads exactly `buf.len()` bytes starting at `offset`.
    ///
    /// # Errors
    ///
    /// [`io::ErrorKind::UnexpectedEof`] if the file ends first (it shrank), and other I/O
    /// errors if it can't be read.
    fn read_exact_at(&self, buf: &mut [u8], offset: u64) -> io::Result<()>;
}

impl ReadAt for File {
    fn size(&self) -> io::Result<u64> {
        Ok(self.metadata()?.len())
    }

    fn modified(&self) -> io::Result<Option<SystemTime>> {
        Ok(self.metadata()?.modified().ok())
    }

    /// On Unix this doesn't move the file's offset, which an already-open file shares with
    /// every handle cloned from it.
    fn read_exact_at(&self, buf: &mut [u8], offset: u64) -> io::Result<()> {
        #[cfg(unix)]
        {
            std::os::unix::fs::FileExt::read_exact_at(self, buf, offset)
        }
        #[cfg(not(unix))]
        {
            use std::io::{Read, Seek, SeekFrom};
            let mut file = self;
            file.seek(SeekFrom::Start(offset))?;
            file.read_exact(buf)
        }
    }
}
