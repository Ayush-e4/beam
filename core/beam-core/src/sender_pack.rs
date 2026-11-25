//! The sender's small-file pack stream for one connection.
//!
//! Small files ([`is_packable`]) are read whole and packed into pieces of about
//! [`PACK_PIECE_LEN`], each sent as a `CHUNK` with item ID [`PACK_ITEM_ID`]. Piece indices
//! restart at 0 on every connection, matching the receiver.

use std::collections::VecDeque;
use std::fs;

use crate::compress::{encode_chunk, worth_compressing};
use crate::integrity::chunk_hash;
use crate::manifest::{ManifestItem, PACK_ITEM_ID};
use crate::messages::ChunkHeader;
use crate::pack::{Packer, is_packable};
use crate::sender_files::{SourceFile, storage_error};
use crate::{BeamError, Result};

/// Target size of one pack piece before it is cut (1 MiB).
pub const PACK_PIECE_LEN: usize = 1024 * 1024;

/// MIME type used to decide whether a pack piece is worth compressing (it mixes many files).
const PACK_MIME: &str = "application/octet-stream";

/// One pack piece ready to send.
#[derive(Debug)]
pub struct PackPiece {
    /// The `CHUNK` payload.
    pub payload: Vec<u8>,
    /// Non-empty files in this piece and their sizes; the receiver acks each as chunk 0.
    pub files: Vec<(u32, u64)>,
}

/// Small files still to pack on this connection.
#[derive(Debug, Default)]
pub struct PackQueue {
    pending: VecDeque<(ManifestItem, SourceFile)>,
    next_index: u64,
}

impl PackQueue {
    /// Starts a new connection's pack stream with the given files.
    pub fn new(files: impl IntoIterator<Item = (ManifestItem, SourceFile)>) -> Self {
        Self {
            pending: files
                .into_iter()
                .filter(|(item, _)| is_packable(item))
                .collect(),
            next_index: 0,
        }
    }

    /// True once every file has been packed into a piece.
    pub fn is_empty(&self) -> bool {
        self.pending.is_empty()
    }

    /// Reads and packs files until a piece is full or none are left.
    ///
    /// # Errors
    ///
    /// Returns [`BeamError::SourceChanged`] if a file changed since the offer, and storage
    /// errors if it cannot be read.
    pub fn next_piece(&mut self) -> Result<Option<PackPiece>> {
        let mut packer = Packer::new();
        let mut files = Vec::new();
        while packer.pending() < PACK_PIECE_LEN {
            let Some((item, source)) = self.pending.pop_front() else {
                break;
            };
            let data = read_small_file(&source)?;
            packer.push(&item, &data)?;
            if item.size > 0 {
                files.push((item.item_id, item.size));
            }
        }
        if packer.pending() == 0 {
            return Ok(None);
        }
        let raw = packer.take();
        let raw_len = u32::try_from(raw.len())
            .map_err(|_| BeamError::Malformed("pack piece is too large".into()))?;
        let encoded = encode_chunk(&raw, worth_compressing(PACK_MIME, &raw))?;
        let header = ChunkHeader {
            item_id: PACK_ITEM_ID,
            index: self.next_index,
            encoding: encoded.encoding,
            raw_len,
            hash: chunk_hash(&raw),
            root: None,
        };
        self.next_index += 1;
        Ok(Some(PackPiece {
            payload: header.encode(&encoded.bytes),
            files,
        }))
    }
}

fn read_small_file(source: &SourceFile) -> Result<Vec<u8>> {
    source.ensure_unchanged()?;
    let data = fs::read(&source.path).map_err(|err| storage_error(&source.path, &err))?;
    if u64::try_from(data.len()).ok() == Some(source.size) {
        Ok(data)
    } else {
        Err(source.changed())
    }
}
