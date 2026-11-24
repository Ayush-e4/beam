//! Transfer manifests: the list of items a sender offers in an `OFFER` frame.
//!
//! A manifest arrives from a peer, so it is validated as a whole before anything is written to
//! disk: every path must be safe, item IDs and paths must be unique (case-insensitively, since
//! many file systems ignore case), no file may also be used as a folder, chunk sizes must be
//! sane and the declared total must match the items.

use std::collections::HashSet;

use crate::safe_path::SafePath;
use crate::{BeamError, Result};

/// Smallest allowed chunk size (64 KiB).
pub const MIN_CHUNK_SIZE: u32 = 64 * 1024;

/// Largest allowed chunk size (16 MiB, the frame payload limit).
pub const MAX_CHUNK_SIZE: u32 = 16 * 1024 * 1024;

/// Most items accepted in a single transfer.
pub const MAX_ITEMS: usize = 1_000_000;

/// Unique identifier of a transfer, chosen by the sender.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TransferId(pub [u8; 16]);

/// One file in a transfer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManifestItem {
    /// Identifier of the item within the transfer.
    pub item_id: u32,
    /// Where the file goes, relative to the receiver's chosen folder.
    pub path: SafePath,
    /// File size in bytes.
    pub size: u64,
    /// MIME type reported by the sender (informational only, never trusted for security).
    pub mime: String,
    /// Last-modified time in milliseconds since the Unix epoch, if known.
    pub modified_at_ms: Option<i64>,
    /// Chunk size used for this file.
    pub chunk_size: u32,
    /// BLAKE3 hash of the whole file.
    pub root_hash: [u8; 32],
}

impl ManifestItem {
    /// Number of chunks the file is split into (0 for an empty file).
    #[must_use]
    pub fn chunk_count(&self) -> u64 {
        self.size.div_ceil(u64::from(self.chunk_size))
    }
}

/// A validated transfer manifest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Manifest {
    transfer_id: TransferId,
    items: Vec<ManifestItem>,
    total_size: u64,
}

impl Manifest {
    /// Validates the items and builds a manifest.
    ///
    /// # Errors
    ///
    /// Returns [`BeamError::Malformed`] if the manifest is empty, too large, has duplicate IDs
    /// or paths, uses a file as a folder, has an invalid chunk size, or if `declared_total`
    /// does not match the sum of item sizes.
    pub fn new(
        transfer_id: TransferId,
        items: Vec<ManifestItem>,
        declared_total: u64,
    ) -> Result<Self> {
        if items.is_empty() {
            return malformed("manifest has no items");
        }
        if items.len() > MAX_ITEMS {
            return malformed("manifest has too many items");
        }

        let mut ids = HashSet::with_capacity(items.len());
        let mut files = HashSet::with_capacity(items.len());
        let mut total: u64 = 0;
        for item in &items {
            if !(MIN_CHUNK_SIZE..=MAX_CHUNK_SIZE).contains(&item.chunk_size) {
                return malformed("item has an invalid chunk size");
            }
            if !ids.insert(item.item_id) {
                return malformed("manifest has a duplicate item id");
            }
            if !files.insert(item.path.collision_key()) {
                return malformed("manifest has duplicate paths");
            }
            total = total
                .checked_add(item.size)
                .ok_or_else(|| BeamError::Malformed("total size overflows".into()))?;
        }
        if total != declared_total {
            return malformed("declared total size does not match the items");
        }
        ensure_no_file_is_a_folder(&items, &files)?;

        Ok(Self {
            transfer_id,
            items,
            total_size: total,
        })
    }

    /// The transfer this manifest belongs to.
    #[must_use]
    pub fn transfer_id(&self) -> TransferId {
        self.transfer_id
    }

    /// The items, in the sender's order.
    #[must_use]
    pub fn items(&self) -> &[ManifestItem] {
        &self.items
    }

    /// Total bytes across all items.
    #[must_use]
    pub fn total_size(&self) -> u64 {
        self.total_size
    }

    /// Looks up an item by its ID.
    #[must_use]
    pub fn item(&self, item_id: u32) -> Option<&ManifestItem> {
        self.items.iter().find(|item| item.item_id == item_id)
    }
}

/// Rejects manifests where one item's path is a folder prefix of another's, e.g. a file `a`
/// alongside `a/b.txt`, which would make the receiver overwrite or fail half-way.
fn ensure_no_file_is_a_folder(items: &[ManifestItem], files: &HashSet<String>) -> Result<()> {
    for item in items {
        let segments = item.path.segments();
        let mut prefix = String::new();
        for segment in &segments[..segments.len().saturating_sub(1)] {
            if !prefix.is_empty() {
                prefix.push('/');
            }
            prefix.push_str(&segment.to_lowercase());
            if files.contains(&prefix) {
                return malformed("a file in the manifest is also used as a folder");
            }
        }
    }
    Ok(())
}

fn malformed<T>(reason: &str) -> Result<T> {
    Err(BeamError::Malformed(reason.to_owned()))
}

#[cfg(test)]
#[path = "manifest_tests.rs"]
mod tests;
