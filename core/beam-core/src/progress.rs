//! Live transfer progress that another task can read while the transfer runs.
//!
//! [`Sender`](crate::Sender) counts bytes the receiver acknowledged (or already had when a
//! transfer resumed); [`Receiver`](crate::Receiver) counts bytes verified and written, including
//! those recovered from its journal. Both count file bytes, not bytes on the wire, so
//! compression and packing don't skew the numbers.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::ChunkSet;
use crate::manifest::ManifestItem;

/// A shared, cheap-to-clone view of one transfer's progress.
#[derive(Debug, Clone, Default)]
pub struct Progress(Arc<Counters>);

#[derive(Debug, Default)]
struct Counters {
    done: AtomicU64,
    total: AtomicU64,
}

impl Progress {
    /// File bytes completed so far.
    #[must_use]
    pub fn bytes_done(&self) -> u64 {
        self.0.done.load(Ordering::Relaxed)
    }

    /// File bytes in the transfer (the selected items only).
    #[must_use]
    pub fn bytes_total(&self) -> u64 {
        self.0.total.load(Ordering::Relaxed)
    }

    pub(crate) fn reset(&self, done: u64, total: u64) {
        self.0.total.store(total, Ordering::Relaxed);
        self.0.done.store(done.min(total), Ordering::Relaxed);
    }

    pub(crate) fn add(&self, bytes: u64) {
        let total = self.bytes_total();
        // Only the owning sender or receiver writes, so load-then-store cannot race.
        let done = self.bytes_done().saturating_add(bytes).min(total);
        self.0.done.store(done, Ordering::Relaxed);
    }
}

/// File bytes of `item` covered by `have` (`None` means none of it).
pub(crate) fn bytes_held(item: &ManifestItem, have: Option<&ChunkSet>) -> u64 {
    let Some(have) = have else { return 0 };
    if have.is_complete() {
        return item.size;
    }
    let Ok(layout) = item.layout() else { return 0 };
    have.ranges()
        .into_iter()
        .flatten()
        .filter_map(|index| layout.range(index).ok())
        .map(|range| range.end - range.start)
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SafePath;

    fn item(size: u64, chunk_size: u32) -> ManifestItem {
        ManifestItem {
            item_id: 0,
            path: SafePath::parse("a.bin").unwrap(),
            size,
            mime: "application/octet-stream".into(),
            modified_at_ms: None,
            chunk_size,
        }
    }

    #[test]
    fn counts_only_the_chunks_held() {
        let item = item(10, 4); // chunks of 4, 4 and 2 bytes
        let mut have = ChunkSet::new(3).unwrap();
        assert_eq!(bytes_held(&item, None), 0);
        assert_eq!(bytes_held(&item, Some(&have)), 0);
        have.insert(2).unwrap();
        assert_eq!(bytes_held(&item, Some(&have)), 2);
        have.insert(0).unwrap();
        assert_eq!(bytes_held(&item, Some(&have)), 6);
        have.insert(1).unwrap();
        assert_eq!(bytes_held(&item, Some(&have)), 10);
    }

    #[test]
    fn done_never_passes_the_total() {
        let progress = Progress::default();
        progress.reset(3, 10);
        progress.add(5);
        assert_eq!(progress.bytes_done(), 8);
        progress.add(50);
        assert_eq!(progress.bytes_done(), 10);
        let view = progress.clone();
        progress.reset(20, 10);
        assert_eq!((view.bytes_done(), view.bytes_total()), (10, 10));
    }
}
