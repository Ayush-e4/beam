//! Crash-safe resume journal for an incoming transfer.
//!
//! The journal is an append-only file next to the partially received data:
//!
//! ```text
//! header : magic "BEAMJNL1" (8) | manifest fingerprint (32) | checksum (8)
//! record : item_id u32 LE (4)   | chunk index u64 LE (8)    | checksum (8)
//! ```
//!
//! Checksums are BLAKE3 (key-derivation mode) over the entry bytes plus the manifest
//! fingerprint, truncated to 8 bytes, so a record can only belong to this journal.
//!
//! **Contract:** call [`Journal::record`] only after the chunk data itself has been flushed to
//! disk (for example with `File::sync_data`). The journal then never claims a chunk that is not
//! durably stored. If the process dies mid-append, the torn last record fails its checksum and
//! is cut off the next time the journal is opened.

use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;

use crate::chunk_set::ChunkSet;
use crate::integrity::Hash;
use crate::manifest::Manifest;
use crate::{BeamError, Result};

const MAGIC: &[u8; 8] = b"BEAMJNL1";
const HEADER_LEN: usize = 48;
const RECORD_LEN: usize = 20;
const CHECKSUM_CONTEXT: &str = "beam 2026-11 journal checksum v1";

/// Resume journal for one transfer.
#[derive(Debug)]
pub struct Journal {
    file: File,
    fingerprint: Hash,
    progress: HashMap<u32, ChunkSet>,
    resumed: bool,
}

impl Journal {
    /// Opens the journal at `path` for `manifest`, resuming previous progress when the journal
    /// matches the manifest, or starting fresh when it is missing, damaged or for a different
    /// manifest.
    ///
    /// # Errors
    ///
    /// Returns a storage error if the file cannot be read, written or synced.
    pub fn open(path: &Path, manifest: &Manifest) -> Result<Self> {
        let fingerprint = manifest.fingerprint();
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path)
            .map_err(BeamError::storage)?;
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes).map_err(BeamError::storage)?;

        let progress = empty_progress(manifest)?;
        let mut journal = Self {
            file,
            fingerprint,
            progress,
            resumed: false,
        };
        if bytes.get(..HEADER_LEN) == Some(&journal.header()[..]) {
            let good_len = journal.replay(&bytes[HEADER_LEN..]);
            journal.resumed = true;
            journal.truncate_to(HEADER_LEN + good_len)?;
        } else {
            journal.reset()?;
        }
        Ok(journal)
    }

    /// True if previous progress was recovered from disk.
    #[must_use]
    pub fn resumed(&self) -> bool {
        self.resumed
    }

    /// Verified chunks of an item, if the item is part of the transfer.
    #[must_use]
    pub fn progress(&self, item_id: u32) -> Option<&ChunkSet> {
        self.progress.get(&item_id)
    }

    /// True once every chunk of every item has been recorded.
    #[must_use]
    pub fn is_complete(&self) -> bool {
        self.progress.values().all(ChunkSet::is_complete)
    }

    /// Durably records verified chunks. See the module docs for the ordering contract.
    ///
    /// # Errors
    ///
    /// Returns [`BeamError::Malformed`] for unknown items or out-of-range chunks (nothing is
    /// written in that case), or a storage error if appending or syncing fails.
    pub fn record(&mut self, chunks: &[(u32, u64)]) -> Result<()> {
        for &(item_id, chunk) in chunks {
            let set = self.progress.get(&item_id).ok_or_else(|| {
                BeamError::Malformed(format!("item {item_id} is not in the transfer"))
            })?;
            if chunk >= set.len() {
                return Err(BeamError::Malformed(format!(
                    "chunk {chunk} is out of range"
                )));
            }
        }

        let mut buffer = Vec::with_capacity(chunks.len() * RECORD_LEN);
        for &(item_id, chunk) in chunks {
            buffer.extend_from_slice(&self.record_bytes(item_id, chunk));
        }
        self.file
            .seek(SeekFrom::End(0))
            .map_err(BeamError::storage)?;
        self.file.write_all(&buffer).map_err(BeamError::storage)?;
        self.file.sync_data().map_err(BeamError::storage)?;

        for &(item_id, chunk) in chunks {
            if let Some(set) = self.progress.get_mut(&item_id) {
                set.insert(chunk)?;
            }
        }
        Ok(())
    }

    /// Applies valid records and returns the byte length of the valid prefix.
    fn replay(&mut self, records: &[u8]) -> usize {
        let mut good = 0;
        for record in records.as_chunks::<RECORD_LEN>().0 {
            let Some((item_id, chunk)) = self.parse_record(record) else {
                break;
            };
            let Some(set) = self.progress.get_mut(&item_id) else {
                break;
            };
            if set.insert(chunk).is_err() {
                break;
            }
            good += RECORD_LEN;
        }
        good
    }

    fn parse_record(&self, record: &[u8; RECORD_LEN]) -> Option<(u32, u64)> {
        let item_id = u32::from_le_bytes(record.get(..4)?.try_into().ok()?);
        let chunk = u64::from_le_bytes(record.get(4..12)?.try_into().ok()?);
        (self.record_bytes(item_id, chunk)[..] == *record).then_some((item_id, chunk))
    }

    fn record_bytes(&self, item_id: u32, chunk: u64) -> [u8; RECORD_LEN] {
        let mut bytes = [0u8; RECORD_LEN];
        bytes[..4].copy_from_slice(&item_id.to_le_bytes());
        bytes[4..12].copy_from_slice(&chunk.to_le_bytes());
        let checksum = self.checksum(&bytes[..12]);
        bytes[12..].copy_from_slice(&checksum);
        bytes
    }

    fn header(&self) -> [u8; HEADER_LEN] {
        let mut bytes = [0u8; HEADER_LEN];
        bytes[..8].copy_from_slice(MAGIC);
        bytes[8..40].copy_from_slice(&self.fingerprint);
        let checksum = self.checksum(&bytes[..40]);
        bytes[40..].copy_from_slice(&checksum);
        bytes
    }

    fn checksum(&self, data: &[u8]) -> [u8; 8] {
        let mut hasher = blake3::Hasher::new_derive_key(CHECKSUM_CONTEXT);
        hasher.update(&self.fingerprint);
        hasher.update(data);
        let mut out = [0u8; 8];
        out.copy_from_slice(&hasher.finalize().as_bytes()[..8]);
        out
    }

    fn reset(&mut self) -> Result<()> {
        self.truncate_to(0)?;
        let header = self.header();
        self.file
            .seek(SeekFrom::Start(0))
            .map_err(BeamError::storage)?;
        self.file.write_all(&header).map_err(BeamError::storage)?;
        self.file.sync_all().map_err(BeamError::storage)
    }

    fn truncate_to(&mut self, len: usize) -> Result<()> {
        let len =
            u64::try_from(len).map_err(|_| BeamError::Malformed("journal too large".into()))?;
        self.file.set_len(len).map_err(BeamError::storage)?;
        self.file.sync_all().map_err(BeamError::storage)
    }
}

fn empty_progress(manifest: &Manifest) -> Result<HashMap<u32, ChunkSet>> {
    manifest
        .items()
        .iter()
        .map(|item| Ok((item.item_id, ChunkSet::new(item.layout()?.chunk_count())?)))
        .collect()
}

#[cfg(test)]
#[path = "journal_tests.rs"]
mod tests;
