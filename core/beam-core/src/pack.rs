//! Small-file packing: many small files travel as one continuous stream, so the cost per file
//! is a 40-byte entry instead of a round of frames, acks and journal writes.
//!
//! ```text
//! entry : item_id u32 LE (4) | length u32 LE (4) | file bytes (`length`) | chunk hash (32)
//! ```
//!
//! A file is packable when it fits in one chunk of its own layout and is at most
//! [`MAX_PACKED_FILE`]. Its entry therefore matches chunk 0 of the normal per-file scheme: the
//! hash is the usual chunk hash, the root hash follows from it, and the resume journal records
//! it as chunk 0. Entries are checked against the manifest (known, packable, right size, sent
//! once) and verified before they are handed out. The pack stream is cut into `CHUNK` frames on
//! its own stream, so compression applies to it like any other data.

use std::collections::HashMap;

use crate::integrity::{Hash, chunk_hash};
use crate::manifest::{Manifest, ManifestItem};
use crate::{BeamError, Result};

/// Largest file that is packed rather than sent on its own.
pub const MAX_PACKED_FILE: u32 = 256 * 1024;
/// Bytes of framing around each packed file.
pub const ENTRY_OVERHEAD: usize = HEADER_LEN + HASH_LEN;

const HEADER_LEN: usize = 8;
const HASH_LEN: usize = 32;

/// Whether `item` travels in the pack stream.
#[must_use]
pub fn is_packable(item: &ManifestItem) -> bool {
    item.size <= u64::from(MAX_PACKED_FILE.min(item.chunk_size))
}

/// Appends small files to a pack stream.
#[derive(Debug, Default)]
pub struct Packer {
    out: Vec<u8>,
}

impl Packer {
    /// Creates an empty packer.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds one file's contents as the entry for `item`.
    ///
    /// # Errors
    ///
    /// Returns [`BeamError::Malformed`] if the item is not packable or `data` does not match
    /// its declared size (for example, the file changed after the manifest was built).
    pub fn push(&mut self, item: &ManifestItem, data: &[u8]) -> Result<()> {
        if !is_packable(item) {
            return Err(BeamError::Malformed(format!(
                "item {} is too large to pack",
                item.item_id
            )));
        }
        let len = u32::try_from(data.len())
            .ok()
            .filter(|len| u64::from(*len) == item.size)
            .ok_or_else(|| {
                BeamError::Malformed(format!(
                    "item {} is {} bytes but the manifest says {}",
                    item.item_id,
                    data.len(),
                    item.size
                ))
            })?;
        self.out.reserve(ENTRY_OVERHEAD + data.len());
        self.out.extend_from_slice(&item.item_id.to_le_bytes());
        self.out.extend_from_slice(&len.to_le_bytes());
        self.out.extend_from_slice(data);
        self.out.extend_from_slice(&chunk_hash(data));
        Ok(())
    }

    /// Bytes packed so far and not yet taken.
    #[must_use]
    pub fn pending(&self) -> usize {
        self.out.len()
    }

    /// Takes everything packed so far, for example once it reaches a chunk's worth.
    pub fn take(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.out)
    }
}

/// A verified file recovered from the pack stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackedFile {
    /// The manifest item this file belongs to.
    pub item_id: u32,
    /// The file contents.
    pub data: Vec<u8>,
    /// Its chunk hash, for the journal and root hash.
    pub hash: Hash,
}

/// Recovers files from a pack stream, however it is split.
#[derive(Debug)]
pub struct Unpacker {
    /// Packable items not yet received, with their sizes.
    expected: HashMap<u32, u32>,
    buf: Vec<u8>,
}

impl Unpacker {
    /// Prepares to receive the packable items of `manifest`.
    #[must_use]
    pub fn new(manifest: &Manifest) -> Self {
        let expected = manifest
            .items()
            .iter()
            .filter(|item| is_packable(item))
            .filter_map(|item| Some((item.item_id, u32::try_from(item.size).ok()?)))
            .collect();
        Self {
            expected,
            buf: Vec::new(),
        }
    }

    /// Marks items as already received (from the resume journal) so they are not expected.
    pub fn skip(&mut self, item_id: u32) {
        self.expected.remove(&item_id);
    }

    /// Packable items still outstanding.
    #[must_use]
    pub fn remaining(&self) -> usize {
        self.expected.len()
    }

    /// Feeds newly arrived pack bytes and returns every file they complete.
    ///
    /// # Errors
    ///
    /// Returns [`BeamError::Malformed`] for an unknown, unpackable, repeated or wrongly sized
    /// item, and [`BeamError::IntegrityMismatch`] if a file fails its hash.
    pub fn feed(&mut self, mut bytes: &[u8]) -> Result<Vec<PackedFile>> {
        let mut files = Vec::new();
        while !bytes.is_empty() {
            let take = self.entry_bytes_wanted()?.min(bytes.len());
            self.buf.extend_from_slice(&bytes[..take]);
            bytes = &bytes[take..];
            if self.buf.len() >= HEADER_LEN && self.entry_bytes_wanted()? == 0 {
                files.push(self.finish_entry()?);
            }
        }
        Ok(files)
    }

    /// Confirms the stream ended cleanly, between entries.
    ///
    /// # Errors
    ///
    /// Returns [`BeamError::Malformed`] if a partial entry is left over.
    pub fn finish(&self) -> Result<()> {
        if self.buf.is_empty() {
            Ok(())
        } else {
            Err(BeamError::Malformed("pack stream ends mid-entry".into()))
        }
    }

    /// Bytes still needed to complete the current entry, validating its header once known.
    fn entry_bytes_wanted(&self) -> Result<usize> {
        let Some((item_id, len)) = self.header() else {
            return Ok(HEADER_LEN - self.buf.len());
        };
        match self.expected.get(&item_id) {
            Some(&size) if size == len => {}
            Some(&size) => {
                return Err(BeamError::Malformed(format!(
                    "packed item {item_id} is {len} bytes, expected {size}"
                )));
            }
            None => {
                return Err(BeamError::Malformed(format!(
                    "packed item {item_id} is unknown, too large or already received"
                )));
            }
        }
        let total = ENTRY_OVERHEAD + len as usize;
        Ok(total - self.buf.len())
    }

    fn header(&self) -> Option<(u32, u32)> {
        let header = self.buf.get(..HEADER_LEN)?;
        let item_id = u32::from_le_bytes([header[0], header[1], header[2], header[3]]);
        let len = u32::from_le_bytes([header[4], header[5], header[6], header[7]]);
        Some((item_id, len))
    }

    fn finish_entry(&mut self) -> Result<PackedFile> {
        // Only called once the header was validated and the whole entry has arrived.
        let entry = std::mem::take(&mut self.buf);
        let item_id = u32::from_le_bytes([entry[0], entry[1], entry[2], entry[3]]);
        let (data, sent_hash) = entry[HEADER_LEN..].split_at(entry.len() - ENTRY_OVERHEAD);
        let hash = chunk_hash(data);
        if hash != sent_hash {
            return Err(BeamError::IntegrityMismatch { chunk: 0 });
        }
        self.expected.remove(&item_id);
        Ok(PackedFile {
            item_id,
            data: data.to_vec(),
            hash,
        })
    }
}

#[cfg(test)]
#[path = "pack_tests.rs"]
mod tests;
