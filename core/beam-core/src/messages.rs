//! Wire encoding of the transfer messages that follow an `OFFER`.
//!
//! All integers are little-endian. A connection runs one transfer at a time, so `CHUNK` and
//! `ACK` frames omit the transfer ID; the decision and control messages carry it so a stale
//! reply from an earlier transfer is never applied to a new one.
//!
//! ```text
//! ACCEPT : transfer_id (16) | count u32 | item_id u32 * count      (count 0 = every item)
//! REJECT : transfer_id (16) | reason u8
//! RESUME : transfer_id (16) | count u32 | item_id u32 * count      (selection, as in ACCEPT)
//!          | count u32 | range * count                             (verified chunks it has)
//! ACK    : count u32 | range * count                               (newly verified chunks)
//! CANCEL : transfer_id (16) | reason u8
//! CHUNK  : item_id u32 | index u64 | encoding u8 | raw_len u32 | chunk hash (32)
//!          | has_root u8 [| root hash (32)] | body
//! range  : item_id u32 | start u64 | end u64                       (chunk indices, end exclusive)
//! ```

use std::ops::Range;

use crate::compress::ChunkEncoding;
use crate::integrity::Hash;
use crate::manifest::TransferId;
use crate::wire::{Reader, Writer};
use crate::{BeamError, Result};

const RANGE_LEN: usize = 4 + 8 + 8;

/// Chunks `start..end` of one item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChunkRange {
    /// The manifest item (or the pack stream).
    pub item_id: u32,
    /// Chunk indices, end exclusive.
    pub chunks: Range<u64>,
}

/// Receiver accepts an offer, optionally only some of its items.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Accept {
    /// The accepted transfer.
    pub transfer_id: TransferId,
    /// Items wanted; empty means every item.
    pub items: Vec<u32>,
}

/// Receiver resumes an interrupted transfer. It repeats the selection, because the sender may
/// never have seen the `ACCEPT` (or may have restarted), and lists what it already has.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resume {
    /// The resumed transfer.
    pub transfer_id: TransferId,
    /// Items wanted; empty means every item.
    pub items: Vec<u32>,
    /// Verified chunks the receiver already has.
    pub have: Vec<ChunkRange>,
}

/// Why a receiver declined or a side cancelled.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum Reason {
    /// The user said no or cancelled.
    UserDeclined = 1,
    /// Not enough free space.
    NoSpace = 2,
    /// Already busy with another transfer.
    Busy = 3,
    /// An unrecoverable error.
    Failed = 4,
}

impl TryFrom<u8> for Reason {
    type Error = BeamError;

    fn try_from(value: u8) -> Result<Self> {
        Ok(match value {
            1 => Self::UserDeclined,
            2 => Self::NoSpace,
            3 => Self::Busy,
            4 => Self::Failed,
            other => return Err(BeamError::Malformed(format!("unknown reason {other}"))),
        })
    }
}

/// Header of a `CHUNK` frame; the chunk body follows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChunkHeader {
    /// The manifest item, or [`crate::manifest::PACK_ITEM_ID`] for the pack stream.
    pub item_id: u32,
    /// Chunk index within the item (or piece number within the pack stream).
    pub index: u64,
    /// How the body is encoded.
    pub encoding: ChunkEncoding,
    /// Length of the body once decoded.
    pub raw_len: u32,
    /// Hash of the decoded body.
    pub hash: Hash,
    /// The item's root hash, sent with its last chunk.
    pub root: Option<Hash>,
}

impl Accept {
    /// Encodes the payload.
    ///
    /// # Errors
    ///
    /// Returns [`BeamError::Malformed`] if there are more than `u32::MAX` items.
    pub fn encode(&self) -> Result<Vec<u8>> {
        let mut w = Writer::new();
        w.bytes(&self.transfer_id.0).count(self.items.len())?;
        for id in &self.items {
            w.u32(*id);
        }
        Ok(w.finish())
    }

    /// Decodes a payload.
    ///
    /// # Errors
    ///
    /// Returns [`BeamError::Malformed`] for a truncated or oversized payload.
    pub fn decode(payload: &[u8]) -> Result<Self> {
        let mut r = Reader::new(payload);
        let transfer_id = TransferId(r.array()?);
        let count = r.count(4)?;
        let items = (0..count).map(|_| r.u32()).collect::<Result<_>>()?;
        r.finish()?;
        Ok(Self { transfer_id, items })
    }
}

impl Resume {
    /// Encodes the payload.
    ///
    /// # Errors
    ///
    /// Returns [`BeamError::Malformed`] if there are more than `u32::MAX` items or ranges.
    pub fn encode(&self) -> Result<Vec<u8>> {
        let mut w = Writer::new();
        w.bytes(&self.transfer_id.0).count(self.items.len())?;
        for id in &self.items {
            w.u32(*id);
        }
        write_ranges(&mut w, &self.have)?;
        Ok(w.finish())
    }

    /// Decodes a payload.
    ///
    /// # Errors
    ///
    /// Returns [`BeamError::Malformed`] for a truncated payload or an empty or reversed range.
    pub fn decode(payload: &[u8]) -> Result<Self> {
        let mut r = Reader::new(payload);
        let transfer_id = TransferId(r.array()?);
        let count = r.count(4)?;
        let items = (0..count).map(|_| r.u32()).collect::<Result<_>>()?;
        let have = read_ranges(&mut r)?;
        r.finish()?;
        Ok(Self {
            transfer_id,
            items,
            have,
        })
    }
}

/// Encodes a `REJECT` or `CANCEL` payload.
#[must_use]
pub fn encode_reason(transfer_id: TransferId, reason: Reason) -> Vec<u8> {
    Writer::new()
        .bytes(&transfer_id.0)
        .u8(reason as u8)
        .finish()
}

/// Decodes a `REJECT` or `CANCEL` payload.
///
/// # Errors
///
/// Returns [`BeamError::Malformed`] for a bad length or unknown reason.
pub fn decode_reason(payload: &[u8]) -> Result<(TransferId, Reason)> {
    let mut r = Reader::new(payload);
    let id = TransferId(r.array()?);
    let reason = Reason::try_from(r.u8()?)?;
    r.finish()?;
    Ok((id, reason))
}

/// Encodes an `ACK` payload.
///
/// # Errors
///
/// Returns [`BeamError::Malformed`] if there are more than `u32::MAX` ranges.
pub fn encode_ack(ranges: &[ChunkRange]) -> Result<Vec<u8>> {
    let mut w = Writer::new();
    write_ranges(&mut w, ranges)?;
    Ok(w.finish())
}

/// Decodes an `ACK` payload.
///
/// # Errors
///
/// Returns [`BeamError::Malformed`] for a truncated payload or an empty or reversed range.
pub fn decode_ack(payload: &[u8]) -> Result<Vec<ChunkRange>> {
    let mut r = Reader::new(payload);
    let ranges = read_ranges(&mut r)?;
    r.finish()?;
    Ok(ranges)
}

impl ChunkHeader {
    /// Encodes the header followed by `body`.
    #[must_use]
    pub fn encode(&self, body: &[u8]) -> Vec<u8> {
        let mut w = Writer::new();
        w.u32(self.item_id)
            .u64(self.index)
            .u8(self.encoding as u8)
            .u32(self.raw_len)
            .bytes(&self.hash);
        match &self.root {
            Some(root) => w.u8(1).bytes(root),
            None => w.u8(0),
        };
        w.bytes(body).finish()
    }

    /// Splits a `CHUNK` payload into its header and body.
    ///
    /// # Errors
    ///
    /// Returns [`BeamError::Malformed`] for a truncated header or unknown encoding.
    pub fn decode(payload: &[u8]) -> Result<(Self, &[u8])> {
        let mut r = Reader::new(payload);
        let header = Self {
            item_id: r.u32()?,
            index: r.u64()?,
            encoding: ChunkEncoding::try_from(r.u8()?)?,
            raw_len: r.u32()?,
            hash: r.array()?,
            root: if r.bool()? { Some(r.array()?) } else { None },
        };
        Ok((header, r.rest()))
    }
}

fn write_ranges(w: &mut Writer, ranges: &[ChunkRange]) -> Result<()> {
    w.count(ranges.len())?;
    for range in ranges {
        w.u32(range.item_id)
            .u64(range.chunks.start)
            .u64(range.chunks.end);
    }
    Ok(())
}

fn read_ranges(r: &mut Reader<'_>) -> Result<Vec<ChunkRange>> {
    let count = r.count(RANGE_LEN)?;
    (0..count)
        .map(|_| {
            let item_id = r.u32()?;
            let (start, end) = (r.u64()?, r.u64()?);
            if start >= end {
                return Err(BeamError::Malformed("empty or reversed chunk range".into()));
            }
            Ok(ChunkRange {
                item_id,
                chunks: start..end,
            })
        })
        .collect()
}

#[cfg(test)]
#[path = "messages_tests.rs"]
mod tests;
