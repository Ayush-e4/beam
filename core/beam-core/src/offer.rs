//! Wire encoding of the transfer manifest carried by `OFFER` frames.
//!
//! ```text
//! manifest : version u8 (1) | transfer_id (16) | total_size u64 | item count u32 | item*
//! item     : item_id u32 | size u64 | chunk_size u32 | has_modified u8 [| modified_at_ms i64]
//!            | path_len u16 | path (UTF-8) | mime_len u8 | mime (UTF-8)
//! frame    : flags u8 (bit 0: last piece) | up to OFFER_PIECE_LEN bytes of the manifest
//! ```
//!
//! A manifest with many files can exceed one frame, so it is split across `OFFER` frames on the
//! control stream and reassembled by [`OfferAssembler`], which caps the total at
//! [`MAX_OFFER_LEN`] and then validates it as a whole with [`Manifest::new`].

use crate::frame::{Frame, FrameType};
use crate::manifest::{Manifest, ManifestItem, TransferId};
use crate::safe_path::SafePath;
use crate::wire::{Reader, Writer};
use crate::{BeamError, Result};

/// Largest encoded manifest accepted (64 MiB, about a million items with typical paths).
pub const MAX_OFFER_LEN: usize = 64 * 1024 * 1024;
/// Manifest bytes carried per `OFFER` frame.
pub const OFFER_PIECE_LEN: usize = 1024 * 1024;
/// Longest MIME type that can be sent.
pub const MAX_MIME_LEN: usize = 255;

const VERSION: u8 = 1;
const LAST_PIECE: u8 = 0b1;
/// Smallest possible encoded item, used to bound a forged item count.
const MIN_ITEM_LEN: usize = 4 + 8 + 4 + 1 + 2 + 1 + 1;

/// Encodes `manifest` as one or more `OFFER` frames for `stream_id`.
///
/// # Errors
///
/// Returns [`BeamError::Malformed`] if a MIME type is too long or the encoded manifest exceeds
/// [`MAX_OFFER_LEN`].
pub fn encode_offer(manifest: &Manifest, stream_id: u32) -> Result<Vec<Frame>> {
    let bytes = encode_manifest(manifest)?;
    let pieces: Vec<&[u8]> = bytes.chunks(OFFER_PIECE_LEN).collect();
    let last = pieces.len() - 1;
    pieces
        .iter()
        .enumerate()
        .map(|(n, piece)| {
            let mut payload = Vec::with_capacity(1 + piece.len());
            payload.push(if n == last { LAST_PIECE } else { 0 });
            payload.extend_from_slice(piece);
            Frame::new(FrameType::Offer, stream_id, payload)
        })
        .collect()
}

fn encode_manifest(manifest: &Manifest) -> Result<Vec<u8>> {
    let mut w = Writer::new();
    w.u8(VERSION)
        .bytes(&manifest.transfer_id().0)
        .u64(manifest.total_size())
        .count(manifest.items().len())?;
    for item in manifest.items() {
        let path = item.path.to_string();
        let path_len = u16::try_from(path.len())
            .map_err(|_| BeamError::Malformed("path too long to encode".into()))?;
        let mime_len = u8::try_from(item.mime.len())
            .map_err(|_| BeamError::Malformed(format!("MIME type longer than {MAX_MIME_LEN}")))?;
        w.u32(item.item_id).u64(item.size).u32(item.chunk_size);
        match item.modified_at_ms {
            Some(ms) => w.u8(1).i64(ms),
            None => w.u8(0),
        };
        w.u16(path_len)
            .bytes(path.as_bytes())
            .u8(mime_len)
            .bytes(item.mime.as_bytes());
    }
    let bytes = w.finish();
    if bytes.len() > MAX_OFFER_LEN {
        return Err(BeamError::Malformed(
            "manifest is too large to offer".into(),
        ));
    }
    Ok(bytes)
}

/// Reassembles a manifest from `OFFER` frames.
#[derive(Debug, Default)]
pub struct OfferAssembler {
    buf: Vec<u8>,
}

impl OfferAssembler {
    /// Creates an empty assembler.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds one `OFFER` frame payload. Returns the validated manifest once the last piece
    /// arrives.
    ///
    /// # Errors
    ///
    /// Returns [`BeamError::Malformed`] if the pieces exceed [`MAX_OFFER_LEN`] or the complete
    /// manifest is invalid.
    pub fn push(&mut self, payload: &[u8]) -> Result<Option<Manifest>> {
        let (&flags, piece) = payload
            .split_first()
            .ok_or_else(|| BeamError::Malformed("empty OFFER frame".into()))?;
        if flags & !LAST_PIECE != 0 {
            return Err(BeamError::Malformed("unknown OFFER flags".into()));
        }
        if self.buf.len() + piece.len() > MAX_OFFER_LEN {
            return Err(BeamError::Malformed("offered manifest is too large".into()));
        }
        self.buf.extend_from_slice(piece);
        if flags & LAST_PIECE == 0 {
            return Ok(None);
        }
        let bytes = std::mem::take(&mut self.buf);
        decode_manifest(&bytes).map(Some)
    }
}

fn decode_manifest(bytes: &[u8]) -> Result<Manifest> {
    let mut r = Reader::new(bytes);
    if r.u8()? != VERSION {
        return Err(BeamError::Malformed("unsupported manifest version".into()));
    }
    let transfer_id = TransferId(r.array()?);
    let total = r.u64()?;
    let count = r.count(MIN_ITEM_LEN)?;
    let mut items = Vec::with_capacity(count);
    for _ in 0..count {
        let item_id = r.u32()?;
        let size = r.u64()?;
        let chunk_size = r.u32()?;
        let modified_at_ms = if r.bool()? { Some(r.i64()?) } else { None };
        let path_len = usize::from(r.u16()?);
        let path = SafePath::parse(r.utf8(path_len)?)?;
        let mime_len = usize::from(r.u8()?);
        let mime = r.utf8(mime_len)?.to_owned();
        items.push(ManifestItem {
            item_id,
            path,
            size,
            mime,
            modified_at_ms,
            chunk_size,
        });
    }
    r.finish()?;
    Manifest::new(transfer_id, items, total)
}

#[cfg(test)]
#[path = "offer_tests.rs"]
mod tests;
