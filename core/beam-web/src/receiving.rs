//! Receiving a transfer in the web app's worker. The phone offers files, the page asks the
//! person, the files land in OPFS ([`crate::opfs_staging`]) and come back as `File`s for the
//! page to save. An interrupted transfer that is offered again resumes where it stopped.

use std::fmt::Write as _;
use std::future::{Future, poll_fn};
use std::pin::pin;

use beam_core::messages::Reason;
use beam_core::{Manifest, Progress, Receiver, read_offer, receive_transfer, reject_offer};
use js_sys::{Array, Date, Function};
use wasm_bindgen::prelude::*;
use web_sys::{Blob, FileSystemDirectoryHandle};

use crate::BeamConnection;
use crate::errors::beam_error;
use crate::opfs_staging::{OpfsStaging, StagedItem, layout, remove_staged, staged_data};

/// How often, at most, the page hears about progress.
const PROGRESS_EVERY_MS: f64 = 100.0;

/// Files the phone offers to send.
#[wasm_bindgen]
pub struct BeamOffer {
    manifest: Manifest,
}

#[wasm_bindgen]
impl BeamOffer {
    /// Identifies the transfer, so a resumed offer can be matched to an earlier one.
    #[wasm_bindgen(getter, js_name = transferId)]
    #[must_use]
    pub fn transfer_id(&self) -> String {
        self.manifest
            .transfer_id()
            .0
            .iter()
            .fold(String::new(), |mut out, byte| {
                let _ = write!(out, "{byte:02x}"); // writing to a String can't fail
                out
            })
    }

    /// Every offered file's size, added up.
    #[wasm_bindgen(getter, js_name = totalBytes)]
    #[must_use]
    pub fn total_bytes(&self) -> f64 {
        js_number(self.manifest.items().iter().map(|item| item.size).sum())
    }

    /// The offered files, by their relative paths (`trip/photos/a.jpg`).
    #[wasm_bindgen(getter)]
    #[must_use]
    pub fn files(&self) -> Vec<OfferedFile> {
        self.manifest
            .items()
            .iter()
            .map(|item| OfferedFile {
                path: item.path.to_string(),
                size: js_number(item.size),
            })
            .collect()
    }
}

/// One offered file.
#[wasm_bindgen(getter_with_clone)]
pub struct OfferedFile {
    /// Its path relative to what the person picked on the phone.
    pub path: String,
    /// Its size in bytes.
    pub size: f64,
}

/// A received file, ready to save.
#[wasm_bindgen(getter_with_clone)]
pub struct ReceivedFile {
    /// Its path relative to what the sender picked.
    pub path: String,
    /// Its bytes, read from OPFS. Readable until [`ReceivedTransfer::remove`].
    pub file: web_sys::File,
}

/// A finished transfer, waiting in OPFS until the page has saved it.
#[wasm_bindgen]
pub struct ReceivedTransfer {
    root: FileSystemDirectoryHandle,
    manifest: Manifest,
    sender: Vec<u8>,
}

#[wasm_bindgen]
impl ReceivedTransfer {
    /// Every received file, in the order offered.
    ///
    /// # Errors
    ///
    /// A storage error if OPFS can't be read.
    pub async fn files(&self) -> Result<Vec<ReceivedFile>, JsValue> {
        let data = staged_data(&self.root, &self.manifest, &[], &self.sender);
        let data = data.await.map_err(beam_error)?;
        let items = layout(&self.manifest, &[]).map_err(beam_error)?;
        items.iter().map(|item| received(&data, item)).collect()
    }

    /// Deletes the transfer from OPFS. Save every file first: their `File`s can't be read
    /// afterwards.
    ///
    /// # Errors
    ///
    /// A storage error if it can't be removed.
    pub async fn remove(&self) -> Result<(), JsValue> {
        remove_staged(&self.root, &self.manifest, &[], &self.sender)
            .await
            .map_err(beam_error)
    }
}

/// The item's slice of `data`, named like the file.
fn received(data: &Blob, item: &StagedItem) -> Result<ReceivedFile, JsValue> {
    let start = js_number(item.offset);
    let bytes = data.slice_with_f64_and_f64(start, start + js_number(item.size))?;
    let name = item.path.rsplit('/').next().unwrap_or(&item.path);
    let file = web_sys::File::new_with_blob_sequence(&Array::of1(&bytes), name)?;
    Ok(ReceivedFile {
        path: item.path.clone(),
        file,
    })
}

#[wasm_bindgen]
impl BeamConnection {
    /// Waits for the phone to offer files.
    ///
    /// # Errors
    ///
    /// A network error if the connection fails, a protocol error for a bad offer.
    #[wasm_bindgen(js_name = readOffer)]
    pub async fn read_offer(&mut self) -> Result<BeamOffer, JsValue> {
        let manifest = read_offer(&mut self.connection).await.map_err(beam_error)?;
        Ok(BeamOffer { manifest })
    }

    /// Says no to `offer`; the phone hears that the person declined.
    ///
    /// # Errors
    ///
    /// A network error if the connection fails.
    pub async fn decline(&mut self, offer: BeamOffer) -> Result<(), JsValue> {
        let transfer_id = offer.manifest.transfer_id();
        reject_offer(&mut self.connection, transfer_id, Reason::UserDeclined)
            .await
            .map_err(beam_error)
    }

    /// Accepts `offer` and receives every file into OPFS under `root` (from
    /// `navigator.storage.getDirectory()`), resuming earlier progress of the same transfer.
    /// `on_progress(done, total)` is called with byte counts as data arrives, at most every
    /// 100 ms and once at the end; if it throws, the transfer carries on.
    ///
    /// # Errors
    ///
    /// A storage error if OPFS fails (or another tab has this transfer open), a network error
    /// if the connection drops, `cancelled` if the phone cancels, and protocol or integrity
    /// errors for bad data.
    pub async fn receive(
        &mut self,
        offer: BeamOffer,
        root: FileSystemDirectoryHandle,
        on_progress: Option<Function>,
    ) -> Result<ReceivedTransfer, JsValue> {
        let sender = self.connection.peer().as_bytes().to_vec();
        let staging = OpfsStaging::open(&root, &offer.manifest, &[], &sender).await;
        let staging = Box::new(staging.map_err(beam_error)?);
        let receiver = Receiver::open_staged(offer.manifest.clone(), &[], staging);
        let receiver = receiver.map_err(beam_error)?;
        let progress = receiver.progress();
        let receiving = receive_transfer(&mut self.connection, receiver);
        reporting(receiving, &progress, on_progress.as_ref())
            .await
            .map_err(beam_error)?;
        Ok(ReceivedTransfer {
            root,
            manifest: offer.manifest,
            sender,
        })
    }
}

/// Runs `future`, telling `report` how far `progress` got whenever the future wakes for new
/// data (at most every [`PROGRESS_EVERY_MS`]) and once when it ends.
async fn reporting<F: Future>(
    future: F,
    progress: &Progress,
    report: Option<&Function>,
) -> F::Output {
    let mut future = pin!(future);
    let mut last = f64::NEG_INFINITY;
    poll_fn(|cx| {
        let polled = future.as_mut().poll(cx);
        if let Some(report) = report {
            let now = Date::now();
            if polled.is_ready() || now - last >= PROGRESS_EVERY_MS {
                last = now;
                let done = JsValue::from(js_number(progress.bytes_done()));
                let total = JsValue::from(js_number(progress.bytes_total()));
                // A throwing callback is the page's bug; the transfer itself is fine.
                let _ = report.call2(&JsValue::NULL, &done, &total);
            }
        }
        polled
    })
    .await
}

/// A byte count as a JavaScript number, exact below 2^53 (8 PiB).
#[allow(clippy::cast_precision_loss)]
const fn js_number(n: u64) -> f64 {
    n as f64
}
