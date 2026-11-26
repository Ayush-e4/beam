//! One file in the browser's origin-private file system (OPFS), read and written through a
//! sync access handle. Only a dedicated worker can open one, and while it is open no other tab
//! can, so a transfer staged here is never written by two tabs at once.

use std::io;

use beam_core::StorageFile;
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::JsFuture;
use web_sys::{
    FileSystemDirectoryHandle, FileSystemFileHandle, FileSystemGetDirectoryOptions,
    FileSystemGetFileOptions, FileSystemReadWriteOptions, FileSystemSyncAccessHandle,
};

/// An open OPFS file. The handle is closed when this is dropped, which lets other tabs in.
#[derive(Debug)]
pub struct OpfsFile {
    handle: FileSystemSyncAccessHandle,
}

impl OpfsFile {
    /// Opens `name` in `directory`, creating it if needed.
    ///
    /// # Errors
    ///
    /// A storage error if the file can't be created, or if another tab has it open.
    pub async fn open(directory: &FileSystemDirectoryHandle, name: &str) -> io::Result<Self> {
        let options = FileSystemGetFileOptions::new();
        options.set_create(true);
        let file: FileSystemFileHandle =
            resolve(directory.get_file_handle_with_options(name, &options)).await?;
        let handle = resolve(file.create_sync_access_handle()).await?;
        Ok(Self { handle })
    }
}

impl StorageFile for OpfsFile {
    fn size(&mut self) -> io::Result<u64> {
        let size = self.handle.get_size().map_err(storage_error)?;
        to_u64(size)
    }

    fn read_exact_at(&mut self, buf: &mut [u8], offset: u64) -> io::Result<()> {
        let mut filled = 0;
        while filled < buf.len() {
            let read = self
                .handle
                .read_with_u8_array_and_options(&mut buf[filled..], &at(offset + filled as u64)?)
                .map_err(storage_error)?;
            let read = usize::try_from(to_u64(read)?).map_err(|_| too_large())?;
            if read == 0 {
                return Err(io::ErrorKind::UnexpectedEof.into());
            }
            filled += read;
        }
        Ok(())
    }

    fn write_all_at(&mut self, data: &[u8], offset: u64) -> io::Result<()> {
        let mut done = 0;
        while done < data.len() {
            let written = self
                .handle
                .write_with_u8_array_and_options(&data[done..], &at(offset + done as u64)?)
                .map_err(storage_error)?;
            let written = usize::try_from(to_u64(written)?).map_err(|_| too_large())?;
            if written == 0 {
                return Err(io::ErrorKind::WriteZero.into());
            }
            done += written;
        }
        Ok(())
    }

    fn set_len(&mut self, len: u64) -> io::Result<()> {
        self.handle
            .truncate_with_f64(to_f64(len)?)
            .map_err(storage_error)
    }

    fn sync(&mut self) -> io::Result<()> {
        self.handle.flush().map_err(storage_error)
    }
}

impl Drop for OpfsFile {
    fn drop(&mut self) {
        self.handle.close();
    }
}

/// The folder `name` in `directory`, created if needed.
///
/// # Errors
///
/// A storage error if it can't be created.
pub async fn folder(
    directory: &FileSystemDirectoryHandle,
    name: &str,
) -> io::Result<FileSystemDirectoryHandle> {
    let options = FileSystemGetDirectoryOptions::new();
    options.set_create(true);
    resolve(directory.get_directory_handle_with_options(name, &options)).await
}

/// Waits for a browser promise and casts what it gives.
///
/// # Errors
///
/// The browser's error as a storage error, or an error if it gives something else.
pub async fn resolve<T: JsCast>(promise: js_sys::Promise) -> io::Result<T> {
    let value = JsFuture::from(promise).await.map_err(storage_error)?;
    value
        .dyn_into()
        .map_err(|_| io::Error::other("the browser gave an unexpected value"))
}

fn at(offset: u64) -> io::Result<FileSystemReadWriteOptions> {
    let options = FileSystemReadWriteOptions::new();
    options.set_at(to_f64(offset)?);
    Ok(options)
}

/// JavaScript numbers hold integers exactly up to 2^53, far past any file.
const MAX_EXACT: u64 = 1 << 53;
const MAX_EXACT_F64: f64 = 9_007_199_254_740_992.0;

#[allow(clippy::cast_precision_loss)] // checked against MAX_EXACT
fn to_f64(n: u64) -> io::Result<f64> {
    if n > MAX_EXACT {
        return Err(too_large());
    }
    Ok(n as f64)
}

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)] // checked first
fn to_u64(n: f64) -> io::Result<u64> {
    if !(0.0..=MAX_EXACT_F64).contains(&n) || n.fract() != 0.0 {
        return Err(io::Error::other("the browser gave an invalid size"));
    }
    Ok(n as u64)
}

fn too_large() -> io::Error {
    io::Error::new(io::ErrorKind::FileTooLarge, "too large for browser storage")
}

/// The browser's error as a storage error of the closest kind.
#[allow(clippy::needless_pass_by_value)] // used as `map_err(storage_error)`
#[must_use]
pub fn storage_error(err: JsValue) -> io::Error {
    let Some(error) = err.dyn_ref::<js_sys::Error>() else {
        return io::Error::other(err.as_string().unwrap_or_else(|| "storage failed".into()));
    };
    let kind = match String::from(error.name()).as_str() {
        "QuotaExceededError" => io::ErrorKind::StorageFull,
        "NotFoundError" => io::ErrorKind::NotFound,
        "NoModificationAllowedError" => io::ErrorKind::ResourceBusy, // open in another tab
        "NotAllowedError" | "SecurityError" => io::ErrorKind::PermissionDenied,
        _ => io::ErrorKind::Other,
    };
    io::Error::new(kind, String::from(error.message()))
}
