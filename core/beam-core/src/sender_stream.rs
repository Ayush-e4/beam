//! Reading one large file as `CHUNK` frames, skipping chunks the receiver already has.
//!
//! The item's root hash goes on its last chunk and covers every chunk hash. When the last chunk
//! still has to be sent, chunks the receiver already has are re-read and hashed (not sent) so
//! the root can be computed. When the receiver already has the last chunk it also has the root,
//! so only the missing chunks are read.

use std::fs::File;
use std::io;

use crate::chunk_set::ChunkSet;
use crate::compress::{SAMPLE_LEN, encode_chunk, worth_compressing};
use crate::integrity::{ChunkLayout, RootHasher, chunk_hash};
use crate::manifest::ManifestItem;
use crate::messages::ChunkHeader;
use crate::sender_files::{SourceFile, storage_error};
use crate::sender_source::read_exact_at;
use crate::{BeamError, Result};

/// One chunk ready to send.
#[derive(Debug)]
pub struct StreamChunk {
    /// The manifest item.
    pub item_id: u32,
    /// Chunk index within the item.
    pub index: u64,
    /// Original (decoded) length, for flow control.
    pub raw_len: u64,
    /// The `CHUNK` payload.
    pub payload: Vec<u8>,
}

/// Streams the missing chunks of one large file.
#[derive(Debug)]
pub struct FileStream {
    item_id: u32,
    source: SourceFile,
    file: File,
    layout: ChunkLayout,
    compress: bool,
    have: Option<ChunkSet>,
    root: Option<RootHasher>,
    next: u64,
    buffer: Vec<u8>,
}

impl FileStream {
    /// Opens `source` to send the chunks of `item` that are not in `have`. Returns `None` when
    /// the receiver already has every chunk.
    ///
    /// # Errors
    ///
    /// Returns [`BeamError::SourceChanged`] if the file changed since the offer, and storage
    /// errors if it cannot be read.
    pub fn open(
        item: &ManifestItem,
        source: SourceFile,
        have: Option<&ChunkSet>,
    ) -> Result<Option<Self>> {
        let layout = item.layout()?;
        let count = layout.chunk_count();
        if have.is_some_and(|set| set.count() >= count) {
            return Ok(None);
        }
        source.ensure_unchanged()?;
        let file = source.open()?;
        let has_last = have.is_some_and(|set| set.contains(count.saturating_sub(1)));
        let mut stream = Self {
            item_id: item.item_id,
            source,
            file,
            layout,
            compress: false,
            have: have.cloned(),
            root: (!has_last).then(|| RootHasher::new(layout)),
            next: 0,
            buffer: Vec::new(),
        };
        let sample_len =
            usize::try_from(layout.size()).map_or(SAMPLE_LEN, |size| size.min(SAMPLE_LEN));
        stream.read_at(0, sample_len)?;
        stream.compress = worth_compressing(&item.mime, &stream.buffer);
        Ok(Some(stream))
    }

    /// Reads the next chunk the receiver is missing. Returns `None` once the file is done.
    ///
    /// # Errors
    ///
    /// Returns [`BeamError::SourceChanged`] if the file changed while it was read, and storage
    /// errors.
    pub fn next_chunk(&mut self) -> Result<Option<StreamChunk>> {
        let count = self.layout.chunk_count();
        while self.next < count {
            let index = self.next;
            self.next += 1;
            let wanted = !self.have.as_ref().is_some_and(|set| set.contains(index));
            if !wanted && self.root.is_none() {
                continue;
            }
            let range = self.layout.range(index)?;
            let len = usize::try_from(range.end - range.start)
                .map_err(|_| BeamError::Malformed("chunk does not fit in memory".into()))?;
            self.read_at(range.start, len)?;
            let hash = chunk_hash(&self.buffer);
            if let Some(root) = &mut self.root {
                root.push(&hash);
            }
            if !wanted {
                continue;
            }
            let is_last = index + 1 == count;
            let root = if is_last {
                self.source.ensure_unchanged()?;
                self.root.as_ref().map(RootHasher::finalize).transpose()?
            } else {
                None
            };
            let encoded = encode_chunk(&self.buffer, self.compress)?;
            let header = ChunkHeader {
                item_id: self.item_id,
                index,
                encoding: encoded.encoding,
                raw_len: u32::try_from(len)
                    .map_err(|_| BeamError::Malformed("chunk is too large".into()))?,
                hash,
                root,
            };
            return Ok(Some(StreamChunk {
                item_id: self.item_id,
                index,
                raw_len: range.end - range.start,
                payload: header.encode(&encoded.bytes),
            }));
        }
        self.source.ensure_unchanged()?;
        Ok(None)
    }

    /// Reads `len` bytes at `offset` into the buffer. A short read means the file shrank.
    fn read_at(&mut self, offset: u64, len: usize) -> Result<()> {
        self.buffer.resize(len, 0);
        let read = read_exact_at(&mut self.file, &mut self.buffer, offset);
        match read {
            Ok(()) => Ok(()),
            Err(err) if err.kind() == io::ErrorKind::UnexpectedEof => Err(self.source.changed()),
            Err(err) => Err(storage_error(&self.source.path, &err)),
        }
    }
}
