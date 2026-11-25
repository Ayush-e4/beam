//! Sending side of a transfer: plan, read, hash, pack, compress and pace chunks.
//!
//! The mirror of [`crate::Receiver`]. Lifecycle, driven by the caller's connection loop:
//!
//! 1. [`Sender::new`] with a [`PreparedTransfer`], then send its manifest as an `OFFER`.
//! 2. On every connection, after the receiver's reply: [`Sender::accept`] for `ACCEPT`, or
//!    [`Sender::resume`] for `RESUME`. Either resets what this connection still has to send.
//! 3. While [`Sender::can_send`]: send [`Sender::next_chunk`] as a `CHUNK` frame. Small files go
//!    first, in the pack stream, then large files chunk by chunk.
//! 4. Feed each `ACK` to [`Sender::handle_ack`], which frees room in the flow-control window.
//!    The transfer is done when [`Sender::is_complete`].
//!
//! Everything here does blocking file I/O; async callers should run it on a blocking thread.

use std::collections::hash_map::Entry;
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};

use crate::chunk_set::ChunkSet;
use crate::manifest::{Manifest, TransferId};
use crate::messages::{Accept, ChunkRange};
use crate::pack::is_packable;
use crate::sender_files::PreparedTransfer;
use crate::sender_pack::PackQueue;
use crate::sender_stream::FileStream;
use crate::{BeamError, Result};

/// Most unacknowledged bytes in flight by default (64 MiB).
pub const DEFAULT_WINDOW: u64 = 64 * 1024 * 1024;

/// Sends one transfer.
#[derive(Debug)]
pub struct Sender {
    prepared: PreparedTransfer,
    window: u64,
    /// Items the receiver accepted; kept across connections because `RESUME` does not repeat it.
    selected: HashSet<u32>,
    /// Chunks the receiver reported verified when this connection began.
    have: HashMap<u32, ChunkSet>,
    pack: PackQueue,
    large: VecDeque<u32>,
    current: Option<FileStream>,
    /// Sent but not yet acknowledged chunks, with their sizes.
    in_flight: BTreeMap<(u32, u64), u64>,
    in_flight_bytes: u64,
    started: bool,
}

impl Sender {
    /// Prepares to send `prepared` with the default flow-control window.
    #[must_use]
    pub fn new(prepared: PreparedTransfer) -> Self {
        Self::with_window(prepared, DEFAULT_WINDOW)
    }

    /// Prepares to send `prepared`, keeping at most `window` unacknowledged bytes in flight
    /// (at least one chunk is always allowed).
    #[must_use]
    pub fn with_window(prepared: PreparedTransfer, window: u64) -> Self {
        let selected = prepared
            .manifest()
            .items()
            .iter()
            .map(|i| i.item_id)
            .collect();
        Self {
            prepared,
            window,
            selected,
            have: HashMap::new(),
            pack: PackQueue::default(),
            large: VecDeque::new(),
            current: None,
            in_flight: BTreeMap::new(),
            in_flight_bytes: 0,
            started: false,
        }
    }

    /// The manifest to offer.
    #[must_use]
    pub fn manifest(&self) -> &Manifest {
        self.prepared.manifest()
    }

    /// Starts a connection after the receiver accepted the offer.
    ///
    /// # Errors
    ///
    /// Returns [`BeamError::Malformed`] if the reply is for another transfer or names an
    /// unknown item.
    pub fn accept(&mut self, accept: &Accept) -> Result<()> {
        self.check_transfer(accept.transfer_id)?;
        let manifest = self.prepared.manifest();
        if let Some(unknown) = accept.items.iter().find(|id| manifest.item(**id).is_none()) {
            return Err(BeamError::Malformed(format!(
                "receiver accepted unknown item {unknown}"
            )));
        }
        self.selected = if accept.items.is_empty() {
            manifest.items().iter().map(|i| i.item_id).collect()
        } else {
            accept.items.iter().copied().collect()
        };
        self.begin_session(HashMap::new());
        Ok(())
    }

    /// Starts a connection after the receiver asked to resume with the chunks it already has.
    /// The selection from the earlier `ACCEPT` still applies.
    ///
    /// # Errors
    ///
    /// Returns [`BeamError::Malformed`] if the reply is for another transfer or lists chunks
    /// the manifest does not have.
    pub fn resume(&mut self, transfer_id: TransferId, have: &[ChunkRange]) -> Result<()> {
        self.check_transfer(transfer_id)?;
        let mut sets: HashMap<u32, ChunkSet> = HashMap::new();
        for range in have {
            let item = self
                .prepared
                .manifest()
                .item(range.item_id)
                .ok_or_else(|| {
                    BeamError::Malformed(format!("resume names unknown item {}", range.item_id))
                })?;
            let count = item.layout()?.chunk_count();
            if range.chunks.end > count {
                return Err(BeamError::Malformed(format!(
                    "resume range is past the end of item {}",
                    range.item_id
                )));
            }
            let set = match sets.entry(range.item_id) {
                Entry::Occupied(entry) => entry.into_mut(),
                Entry::Vacant(entry) => entry.insert(ChunkSet::new(count)?),
            };
            for index in range.chunks.clone() {
                set.insert(index)?;
            }
        }
        self.begin_session(sets);
        Ok(())
    }

    /// True if another chunk fits in the flow-control window.
    #[must_use]
    pub fn can_send(&self) -> bool {
        self.in_flight_bytes < self.window || self.in_flight.is_empty()
    }

    /// Reads the next `CHUNK` payload to send, or `None` once everything has been sent.
    ///
    /// # Errors
    ///
    /// Returns [`BeamError::Malformed`] before a connection started,
    /// [`BeamError::SourceChanged`] if a file changed since the offer, and storage errors.
    pub fn next_chunk(&mut self) -> Result<Option<Vec<u8>>> {
        if !self.started {
            return Err(BeamError::Malformed(
                "the receiver has not accepted yet".into(),
            ));
        }
        if let Some(piece) = self.pack.next_piece()? {
            for (item_id, size) in piece.files {
                self.track(item_id, 0, size);
            }
            return Ok(Some(piece.payload));
        }
        loop {
            if let Some(stream) = &mut self.current {
                if let Some(chunk) = stream.next_chunk()? {
                    self.track(chunk.item_id, chunk.index, chunk.raw_len);
                    return Ok(Some(chunk.payload));
                }
                self.current = None;
                self.large.pop_front();
            }
            let Some(&item_id) = self.large.front() else {
                return Ok(None);
            };
            self.current = self.open_stream(item_id)?;
            if self.current.is_none() {
                self.large.pop_front();
            }
        }
    }

    /// Frees the window for chunks the receiver acknowledged. Unknown or repeated ranges are
    /// ignored.
    pub fn handle_ack(&mut self, ranges: &[ChunkRange]) {
        for range in ranges {
            let span = (range.item_id, range.chunks.start)..(range.item_id, range.chunks.end);
            let acked: Vec<(u32, u64)> = self.in_flight.range(span).map(|(key, _)| *key).collect();
            for key in acked {
                if let Some(size) = self.in_flight.remove(&key) {
                    self.in_flight_bytes = self.in_flight_bytes.saturating_sub(size);
                }
            }
        }
    }

    /// Unacknowledged bytes in flight.
    #[must_use]
    pub fn in_flight(&self) -> u64 {
        self.in_flight_bytes
    }

    /// True once everything has been sent and acknowledged on this connection.
    #[must_use]
    pub fn is_complete(&self) -> bool {
        self.started
            && self.pack.is_empty()
            && self.large.is_empty()
            && self.current.is_none()
            && self.in_flight.is_empty()
    }

    fn check_transfer(&self, transfer_id: TransferId) -> Result<()> {
        if transfer_id == self.prepared.manifest().transfer_id() {
            Ok(())
        } else {
            Err(BeamError::Malformed("reply is for another transfer".into()))
        }
    }

    /// Plans this connection: small files not yet received go in the pack, large files with
    /// missing chunks follow in manifest order.
    fn begin_session(&mut self, have: HashMap<u32, ChunkSet>) {
        let done = |id: u32| have.get(&id).is_some_and(ChunkSet::is_complete);
        let mut small = Vec::new();
        let mut large = VecDeque::new();
        for item in self.prepared.manifest().items() {
            if !self.selected.contains(&item.item_id) || (item.size > 0 && done(item.item_id)) {
                continue;
            }
            if is_packable(item) {
                if let Some(source) = self.prepared.source(item.item_id) {
                    small.push((item.clone(), source.clone()));
                }
            } else {
                large.push_back(item.item_id);
            }
        }
        self.pack = PackQueue::new(small);
        self.large = large;
        self.current = None;
        self.have = have;
        self.in_flight.clear();
        self.in_flight_bytes = 0;
        self.started = true;
    }

    fn open_stream(&self, item_id: u32) -> Result<Option<FileStream>> {
        let missing = || BeamError::Malformed(format!("item {item_id} has no source file"));
        let item = self.prepared.manifest().item(item_id).ok_or_else(missing)?;
        let source = self.prepared.source(item_id).ok_or_else(missing)?;
        FileStream::open(item, source.clone(), self.have.get(&item_id))
    }

    fn track(&mut self, item_id: u32, index: u64, size: u64) {
        if self.in_flight.insert((item_id, index), size).is_none() {
            self.in_flight_bytes = self.in_flight_bytes.saturating_add(size);
        }
    }
}

#[cfg(test)]
#[path = "sender_tests.rs"]
mod tests;
