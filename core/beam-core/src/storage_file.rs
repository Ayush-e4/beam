//! Files the receiver reads and writes by position: staged data and the resume journal. On the
//! phone and the Mac they are ordinary files; in a browser's worker, OPFS sync access handles.

use std::fmt;
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom, Write};

use crate::read_at::MaybeSend;

/// A file the receiver reads and writes at any position.
pub trait StorageFile: MaybeSend + fmt::Debug {
    /// Its size, in bytes.
    ///
    /// # Errors
    ///
    /// If it can't be inspected.
    fn size(&mut self) -> io::Result<u64>;

    /// Reads exactly `buf.len()` bytes at `offset`.
    ///
    /// # Errors
    ///
    /// [`io::ErrorKind::UnexpectedEof`] if the file ends first, other I/O errors if it can't be
    /// read.
    fn read_exact_at(&mut self, buf: &mut [u8], offset: u64) -> io::Result<()>;

    /// Writes all of `data` at `offset`, growing the file if needed.
    ///
    /// # Errors
    ///
    /// If it can't be written (a full disk, say).
    fn write_all_at(&mut self, data: &[u8], offset: u64) -> io::Result<()>;

    /// Cuts or grows the file to `len` bytes.
    ///
    /// # Errors
    ///
    /// If it can't be resized.
    fn set_len(&mut self, len: u64) -> io::Result<()>;

    /// Makes everything written so far durable, including the size.
    ///
    /// # Errors
    ///
    /// If it can't be flushed.
    fn sync(&mut self) -> io::Result<()>;
}

impl StorageFile for File {
    fn size(&mut self) -> io::Result<u64> {
        Ok(self.metadata()?.len())
    }

    fn read_exact_at(&mut self, buf: &mut [u8], offset: u64) -> io::Result<()> {
        self.seek(SeekFrom::Start(offset))?;
        self.read_exact(buf)
    }

    fn write_all_at(&mut self, data: &[u8], offset: u64) -> io::Result<()> {
        self.seek(SeekFrom::Start(offset))?;
        self.write_all(data)
    }

    fn set_len(&mut self, len: u64) -> io::Result<()> {
        Self::set_len(self, len)
    }

    /// `fdatasync`: data plus the metadata needed to read it back, such as the size.
    fn sync(&mut self) -> io::Result<()> {
        self.sync_data()
    }
}
