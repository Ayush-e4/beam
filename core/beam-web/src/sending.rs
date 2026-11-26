//! Sending files the person picked in the browser to the phone, from the web app's worker.
//! Picked `File`s are read by position with `FileReaderSync`, which only workers have.

use std::fmt;
use std::io;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use beam_core::messages::Reason;
use beam_core::{ReadAt, ReadableFile, SendOutcome, Sender, prepare_readable, send_transfer};
use js_sys::{Array, Function, Uint8Array};
use wasm_bindgen::prelude::*;
use web_sys::{File, FileReaderSync};

use crate::BeamConnection;
use crate::errors::{beam_error, invalid};
use crate::progress::{js_number, reporting};

/// A picked file, read by position. A `File` is a snapshot: if the file on disk changes or
/// goes away after it was picked, reading it fails, and the engine reports the source changed.
struct FileReadAt {
    file: File,
    reader: FileReaderSync,
}

impl fmt::Debug for FileReadAt {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FileReadAt")
            .field("name", &self.file.name())
            .finish_non_exhaustive()
    }
}

impl ReadAt for FileReadAt {
    fn size(&self) -> io::Result<u64> {
        whole(self.file.size())
    }

    fn modified(&self) -> io::Result<Option<SystemTime>> {
        let ms = whole(self.file.last_modified())?;
        Ok(SystemTime::UNIX_EPOCH.checked_add(Duration::from_millis(ms)))
    }

    fn read_exact_at(&self, buf: &mut [u8], offset: u64) -> io::Result<()> {
        let start = js_number(offset);
        let slice = self
            .file
            .slice_with_f64_and_f64(start, start + js_number(buf.len() as u64))
            .map_err(read_error)?;
        let bytes = self
            .reader
            .read_as_array_buffer(&slice)
            .map_err(read_error)?;
        let bytes = Uint8Array::new(&bytes);
        if bytes.length() as usize != buf.len() {
            return Err(io::ErrorKind::UnexpectedEof.into()); // the file is shorter than it was
        }
        bytes.copy_to(buf);
        Ok(())
    }
}

/// A JavaScript size or time, which is a whole non-negative number.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)] // checked first
fn whole(n: f64) -> io::Result<u64> {
    if n >= 0.0 && n.fract() == 0.0 && n < 9_007_199_254_740_992.0 {
        Ok(n as u64)
    } else {
        Err(io::Error::other("the browser gave an invalid file size"))
    }
}

/// A failed read of a picked file. Browsers say `NotReadableError` when it changed on disk or
/// went away since it was picked; the engine reports that as the source having changed.
#[allow(clippy::needless_pass_by_value)] // used as `map_err(read_error)`
fn read_error(err: JsValue) -> io::Error {
    let error = err.dyn_ref::<js_sys::Error>();
    let message: String = error.map_or_else(
        || "reading the file failed".to_owned(),
        |e| String::from(e.message()),
    );
    let kind = match error.map(|e| String::from(e.name())).as_deref() {
        Some("NotReadableError" | "NotFoundError") => io::ErrorKind::UnexpectedEof,
        _ => io::ErrorKind::Other,
    };
    io::Error::new(kind, message)
}

#[wasm_bindgen]
impl BeamConnection {
    /// Offers `files` (picked `File`s) to the phone and sends them if the person there
    /// accepts. `on_progress(done, total)` is called with byte counts as the phone confirms
    /// data, at most every 100 ms and once at the end; if it throws, the transfer carries on.
    /// Returns `"delivered"` once the phone saved every file, or why it refused:
    /// `"declined"`, `"no-space"`, `"busy"` or `"failed"`.
    ///
    /// # Errors
    ///
    /// `invalid` if `files` holds something other than `File`s, a network error if the
    /// connection drops, `cancelled` if the phone cancels, and a storage error if a file
    /// changed or went away since it was picked.
    #[wasm_bindgen(js_name = sendFiles)]
    pub async fn send_files(
        &mut self,
        files: Array,
        on_progress: Option<Function>,
    ) -> Result<String, JsValue> {
        let reader = FileReaderSync::new()?;
        let mut readable = Vec::new();
        for file in files.iter() {
            let file: File = file
                .dyn_into()
                .map_err(|_| invalid("files must be Files"))?;
            readable.push(ReadableFile {
                name: file.name(),
                reader: Arc::new(FileReadAt {
                    file,
                    reader: reader.clone(),
                }),
            });
        }
        let sender = Sender::new(prepare_readable(readable).map_err(beam_error)?);
        let progress = sender.progress();
        // `send_transfer` shares the sender with its file work; here there is one thread.
        #[allow(clippy::arc_with_non_send_sync)]
        let sender = Arc::new(Mutex::new(sender));
        let sending = send_transfer(&mut self.connection, &sender);
        let outcome = reporting(sending, &progress, on_progress.as_ref()).await;
        Ok(outcome_name(outcome.map_err(beam_error)?).to_owned())
    }
}

const fn outcome_name(outcome: SendOutcome) -> &'static str {
    match outcome {
        SendOutcome::Delivered => "delivered",
        SendOutcome::Rejected(Reason::UserDeclined) => "declined",
        SendOutcome::Rejected(Reason::NoSpace) => "no-space",
        SendOutcome::Rejected(Reason::Busy) => "busy",
        SendOutcome::Rejected(Reason::Failed) => "failed",
    }
}
