//! Progress and finishing a transfer: what the receiver has, root-hash checks and publishing
//! into the destination.

use std::path::PathBuf;

use super::Receiver;
use crate::integrity::{RootHasher, chunk_hash};
use crate::manifest::ManifestItem;
use crate::messages::ChunkRange;
use crate::pack::is_packable;
use crate::progress::{Progress, bytes_held};
use crate::{BeamError, Result};

impl Receiver {
    /// Bytes verified and written so far (including earlier runs), for showing progress from
    /// another task.
    #[must_use]
    pub fn progress(&self) -> Progress {
        self.progress.clone()
    }

    /// Restarts the count from what the journal holds for the selected items.
    pub(super) fn reset_progress(&self) {
        let selected = || {
            let manifest = &self.manifest;
            manifest
                .items()
                .iter()
                .filter(|i| self.selected.contains(&i.item_id))
        };
        let held = selected()
            .map(|i| bytes_held(i, self.journal.progress(i.item_id)))
            .sum();
        self.progress.reset(held, selected().map(|i| i.size).sum());
    }

    /// Verified chunks already on disk, for a `RESUME` reply.
    #[must_use]
    pub fn have(&self) -> Vec<ChunkRange> {
        let mut ranges = Vec::new();
        for item in self.manifest.items() {
            if let Some(set) = self.journal.progress(item.item_id) {
                ranges.extend(set.ranges().into_iter().map(|chunks| ChunkRange {
                    item_id: item.item_id,
                    chunks,
                }));
            }
        }
        ranges
    }

    /// The items to list in an `ACCEPT`: empty when every item was selected.
    #[must_use]
    pub fn accepted_items(&self) -> Vec<u32> {
        if self.selected.len() == self.manifest.items().len() {
            return Vec::new();
        }
        let mut items: Vec<u32> = self.selected.iter().copied().collect();
        items.sort_unstable();
        items
    }

    /// True once every expected chunk of this session has arrived, even if not yet
    /// checkpointed. The next [`Receiver::checkpoint`] then normally completes the transfer.
    #[must_use]
    pub fn has_all_data(&self) -> bool {
        self.remaining == 0
    }

    /// Verifies every file's root hash and moves the files into place. Returns their final
    /// paths in manifest order.
    ///
    /// # Errors
    ///
    /// Returns [`BeamError::Malformed`] if the transfer is incomplete,
    /// [`BeamError::IntegrityMismatch`] if a file fails its root hash, and storage errors.
    pub fn finish(mut self) -> Result<Vec<PathBuf>> {
        self.checkpoint()?;
        if !self.is_complete() {
            return Err(BeamError::Malformed("transfer is not complete".into()));
        }
        let items: Vec<ManifestItem> = self
            .manifest
            .items()
            .iter()
            .filter(|i| self.selected.contains(&i.item_id))
            .cloned()
            .collect();
        for item in items.iter().filter(|i| !is_packable(i)) {
            self.verify_root(item)?;
        }
        let paths = items
            .iter()
            .map(|item| self.store.publish(item))
            .collect::<Result<Vec<_>>>()?;
        self.store.remove_staging()?;
        Ok(paths)
    }

    fn verify_root(&mut self, item: &ManifestItem) -> Result<()> {
        let layout = item.layout()?;
        let mut hasher = RootHasher::new(layout);
        let known = self.hashes.remove(&item.item_id).unwrap_or_default();
        for index in 0..layout.chunk_count() {
            let seen = usize::try_from(index)
                .ok()
                .and_then(|i| known.get(i).copied().flatten());
            let hash = match seen {
                Some(hash) => hash,
                None => chunk_hash(&self.store.read_range(item.item_id, layout.range(index)?)?),
            };
            hasher.push(&hash);
        }
        if hasher.finalize()? == self.store.load_root(item.item_id)? {
            Ok(())
        } else {
            Err(BeamError::IntegrityMismatch {
                chunk: layout.chunk_count().saturating_sub(1),
            })
        }
    }
}

/// Merges sorted `(item, chunk)` pairs into contiguous ranges.
pub(super) fn coalesce(chunks: &[(u32, u64)]) -> Vec<ChunkRange> {
    let mut ranges: Vec<ChunkRange> = Vec::new();
    for &(item_id, index) in chunks {
        match ranges.last_mut() {
            Some(last) if last.item_id == item_id && last.chunks.end == index => {
                last.chunks.end += 1;
            }
            _ => ranges.push(ChunkRange {
                item_id,
                chunks: index..index + 1,
            }),
        }
    }
    ranges
}
