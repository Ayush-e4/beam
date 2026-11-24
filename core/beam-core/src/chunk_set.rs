//! Compact set of chunk indices, used to track which chunks of a file are verified.
//!
//! Backed by a bitset (one bit per chunk), so a 100 GB file in 4 MiB chunks needs about 3 KB.
//! [`ChunkSet::ranges`] turns the set into contiguous ranges for `ACK` and `RESUME` frames.

use std::ops::Range;

use crate::{BeamError, Result};

const WORD_BITS: u64 = 64;

/// Set of chunk indices in `0..len`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChunkSet {
    words: Vec<u64>,
    len: u64,
    count: u64,
}

impl ChunkSet {
    /// Creates an empty set for a file with `len` chunks.
    ///
    /// # Errors
    ///
    /// Returns [`BeamError::Malformed`] if the bitset would not fit in memory.
    pub fn new(len: u64) -> Result<Self> {
        let words = usize::try_from(len.div_ceil(WORD_BITS))
            .map_err(|_| BeamError::Malformed("too many chunks to track".into()))?;
        Ok(Self {
            words: vec![0; words],
            len,
            count: 0,
        })
    }

    /// Total number of chunks the set covers.
    #[must_use]
    pub fn len(&self) -> u64 {
        self.len
    }

    /// True if the file has no chunks.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Number of chunks in the set.
    #[must_use]
    pub fn count(&self) -> u64 {
        self.count
    }

    /// True once every chunk is in the set.
    #[must_use]
    pub fn is_complete(&self) -> bool {
        self.count == self.len
    }

    /// Adds a chunk. Returns `true` if it was not already present.
    ///
    /// # Errors
    ///
    /// Returns [`BeamError::Malformed`] if `index` is out of range.
    pub fn insert(&mut self, index: u64) -> Result<bool> {
        let (word, mask) = self.locate(index)?;
        let newly_added = *word & mask == 0;
        *word |= mask;
        if newly_added {
            self.count += 1;
        }
        Ok(newly_added)
    }

    /// True if `index` is in the set (out-of-range indices are never present).
    #[must_use]
    pub fn contains(&self, index: u64) -> bool {
        if index >= self.len {
            return false;
        }
        let (word, bit) = split(index);
        self.words.get(word).is_some_and(|w| w & (1 << bit) != 0)
    }

    /// Contiguous ranges of present chunks, in ascending order.
    #[must_use]
    pub fn ranges(&self) -> Vec<Range<u64>> {
        self.collect_ranges(true)
    }

    /// Contiguous ranges of missing chunks, in ascending order.
    #[must_use]
    pub fn missing(&self) -> Vec<Range<u64>> {
        self.collect_ranges(false)
    }

    fn collect_ranges(&self, present: bool) -> Vec<Range<u64>> {
        let mut ranges = Vec::new();
        let mut start = None;
        for index in 0..self.len {
            match (self.contains(index) == present, start) {
                (true, None) => start = Some(index),
                (false, Some(s)) => {
                    ranges.push(s..index);
                    start = None;
                }
                _ => {}
            }
        }
        if let Some(s) = start {
            ranges.push(s..self.len);
        }
        ranges
    }

    fn locate(&mut self, index: u64) -> Result<(&mut u64, u64)> {
        if index >= self.len {
            return Err(BeamError::Malformed(format!(
                "chunk index {index} is out of range"
            )));
        }
        let (word, bit) = split(index);
        let word = self
            .words
            .get_mut(word)
            .ok_or_else(|| BeamError::Malformed("chunk index out of range".into()))?;
        Ok((word, 1 << bit))
    }
}

/// Splits a chunk index into its word index and bit position.
fn split(index: u64) -> (usize, u64) {
    let word = usize::try_from(index / WORD_BITS).unwrap_or(usize::MAX);
    (word, index % WORD_BITS)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tracks_inserted_chunks() {
        let mut set = ChunkSet::new(130).expect("fits");
        assert!(set.insert(0).expect("in range"));
        assert!(set.insert(129).expect("in range"));
        assert!(
            !set.insert(0).expect("in range"),
            "second insert is a no-op"
        );

        assert_eq!(set.count(), 2);
        assert!(set.contains(129) && !set.contains(64) && !set.contains(500));
        assert!(set.insert(130).is_err());
    }

    #[test]
    fn reports_present_and_missing_ranges() {
        let mut set = ChunkSet::new(10).expect("fits");
        for index in [0, 1, 2, 5, 9] {
            set.insert(index).expect("in range");
        }
        assert_eq!(set.ranges(), vec![0..3, 5..6, 9..10]);
        assert_eq!(set.missing(), vec![3..5, 6..9]);
    }

    #[test]
    fn completes_when_every_chunk_is_present() {
        let mut set = ChunkSet::new(3).expect("fits");
        for index in 0..3 {
            assert!(!set.is_complete());
            set.insert(index).expect("in range");
        }
        assert!(set.is_complete());
        assert_eq!(set.missing(), Vec::<Range<u64>>::new());
    }

    #[test]
    fn empty_files_are_complete_immediately() {
        let set = ChunkSet::new(0).expect("fits");
        assert!(set.is_empty() && set.is_complete());
        assert_eq!(set.ranges(), Vec::<Range<u64>>::new());
    }
}
