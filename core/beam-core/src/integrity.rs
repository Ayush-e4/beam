//! Chunk layout and BLAKE3 integrity checks.
//!
//! A file is split into fixed-size chunks (the last one may be shorter). Each chunk is hashed on
//! its own, so the receiver can verify and persist chunks independently and resume after a
//! drop. The item's **root hash** commits to the file size, chunk size and every chunk hash in
//! order, so a receiver that has verified all chunks can confirm the whole file with one
//! comparison. Hashes use BLAKE3 key derivation with distinct contexts so a chunk hash can never
//! be confused with a root hash.

use std::io::Read;
use std::ops::Range;

use crate::{BeamError, ErrorCategory, Result};

const CHUNK_CONTEXT: &str = "beam 2026-11 chunk hash v1";
const ROOT_CONTEXT: &str = "beam 2026-11 root hash v1";

/// A 32-byte BLAKE3 digest.
pub type Hash = [u8; 32];

/// How a file of a given size is split into chunks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChunkLayout {
    size: u64,
    chunk_size: u32,
}

impl ChunkLayout {
    /// Creates a layout.
    ///
    /// # Errors
    ///
    /// Returns [`BeamError::Malformed`] if `chunk_size` is zero.
    pub fn new(size: u64, chunk_size: u32) -> Result<Self> {
        if chunk_size == 0 {
            return Err(BeamError::Malformed("chunk size must not be zero".into()));
        }
        Ok(Self { size, chunk_size })
    }

    /// File size in bytes.
    #[must_use]
    pub fn size(&self) -> u64 {
        self.size
    }

    /// Number of chunks (0 for an empty file).
    #[must_use]
    pub fn chunk_count(&self) -> u64 {
        self.size.div_ceil(u64::from(self.chunk_size))
    }

    /// Byte range of chunk `index` within the file.
    ///
    /// # Errors
    ///
    /// Returns [`BeamError::Malformed`] if `index` is past the last chunk.
    pub fn range(&self, index: u64) -> Result<Range<u64>> {
        if index >= self.chunk_count() {
            return Err(BeamError::Malformed(format!(
                "chunk index {index} is out of range"
            )));
        }
        let start = index * u64::from(self.chunk_size);
        let end = start
            .saturating_add(u64::from(self.chunk_size))
            .min(self.size);
        Ok(start..end)
    }

    /// Verifies a received chunk's length and hash.
    ///
    /// # Errors
    ///
    /// Returns [`BeamError::Malformed`] for an out-of-range index or wrong length, and
    /// [`BeamError::IntegrityMismatch`] if the data does not match `expected`.
    pub fn verify_chunk(&self, index: u64, data: &[u8], expected: &Hash) -> Result<()> {
        let range = self.range(index)?;
        if u64::try_from(data.len()).ok() != Some(range.end - range.start) {
            return Err(BeamError::Malformed(format!(
                "chunk {index} has the wrong length"
            )));
        }
        if &chunk_hash(data) == expected {
            Ok(())
        } else {
            Err(BeamError::IntegrityMismatch { chunk: index })
        }
    }
}

/// Hashes one chunk.
#[must_use]
pub fn chunk_hash(data: &[u8]) -> Hash {
    let mut hasher = blake3::Hasher::new_derive_key(CHUNK_CONTEXT);
    hasher.update(data);
    *hasher.finalize().as_bytes()
}

/// Builds an item's root hash from its chunk hashes, fed in chunk order.
#[derive(Debug, Clone)]
pub struct RootHasher {
    hasher: blake3::Hasher,
    expected_chunks: u64,
    seen_chunks: u64,
}

impl RootHasher {
    /// Starts a root hash for a file with the given layout.
    #[must_use]
    pub fn new(layout: ChunkLayout) -> Self {
        let mut hasher = blake3::Hasher::new_derive_key(ROOT_CONTEXT);
        hasher.update(&layout.size.to_le_bytes());
        hasher.update(&layout.chunk_size.to_le_bytes());
        Self {
            hasher,
            expected_chunks: layout.chunk_count(),
            seen_chunks: 0,
        }
    }

    /// Adds the next chunk hash.
    pub fn push(&mut self, chunk: &Hash) {
        self.hasher.update(chunk);
        self.seen_chunks += 1;
    }

    /// Finishes the root hash.
    ///
    /// # Errors
    ///
    /// Returns [`BeamError::Malformed`] if the number of pushed chunks does not match the layout.
    pub fn finalize(&self) -> Result<Hash> {
        if self.seen_chunks != self.expected_chunks {
            return Err(BeamError::Malformed(format!(
                "root hash needs {} chunks, got {}",
                self.expected_chunks, self.seen_chunks
            )));
        }
        Ok(*self.hasher.finalize().as_bytes())
    }
}

/// Chunk hashes and root hash of a whole file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileHashes {
    /// One hash per chunk, in order.
    pub chunks: Vec<Hash>,
    /// The item's root hash.
    pub root: Hash,
}

/// Reads `reader` to the end and hashes it chunk by chunk with a single reusable buffer.
///
/// # Errors
///
/// Returns a storage I/O error if reading fails, or [`BeamError::Malformed`] if the reader holds
/// more or fewer bytes than `layout` describes.
pub fn hash_reader(mut reader: impl Read, layout: ChunkLayout) -> Result<FileHashes> {
    let chunk_len = usize::try_from(layout.chunk_size)
        .map_err(|_| BeamError::Malformed("chunk size does not fit in memory".into()))?;
    let mut buffer = vec![0u8; chunk_len];
    let mut root = RootHasher::new(layout);
    let mut chunks = Vec::new();

    for index in 0..layout.chunk_count() {
        let range = layout.range(index)?;
        let len = usize::try_from(range.end - range.start)
            .map_err(|_| BeamError::Malformed("chunk does not fit in memory".into()))?;
        let slot = buffer
            .get_mut(..len)
            .ok_or_else(|| BeamError::Malformed("chunk is larger than the chunk size".into()))?;
        reader.read_exact(slot).map_err(BeamError::storage)?;
        let hash = chunk_hash(slot);
        root.push(&hash);
        chunks.push(hash);
    }

    let mut probe = [0u8; 1];
    if reader.read(&mut probe).map_err(BeamError::storage)? != 0 {
        return Err(BeamError::Malformed(
            "file is larger than its declared size".into(),
        ));
    }
    Ok(FileHashes {
        chunks,
        root: root.finalize()?,
    })
}

/// Returns true if an error is an integrity failure (useful for retry decisions).
#[must_use]
pub fn is_integrity_error(error: &BeamError) -> bool {
    error.category() == ErrorCategory::Crypto
}

#[cfg(test)]
#[path = "integrity_tests.rs"]
mod tests;
