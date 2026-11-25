//! Receiving side of a transfer: verify, write, journal and publish.
//!
//! Lifecycle, driven by the caller's connection loop:
//!
//! 1. [`Receiver::open`] once the user accepts an offer. It resumes from the journal if this
//!    transfer was interrupted before.
//! 2. On every connection: [`Receiver::begin_session`], then send `ACCEPT`, or `RESUME` with
//!    [`Receiver::have`] when there is earlier progress.
//! 3. For each `CHUNK` frame: [`Receiver::handle_chunk`]. Every so often (for example every few
//!    chunks or 250 ms): [`Receiver::checkpoint`], and `ACK` the ranges it returns. Data is
//!    flushed before the journal records it, so an ack is a promise the data is on disk.
//! 4. When [`Receiver::is_complete`]: [`Receiver::finish`] checks every file's root hash and
//!    moves the files into place.
//!
//! Large files arrive as chunks on their own; small files arrive in the pack stream
//! ([`crate::pack`]). Each chunk is checked against the manifest and its hash before it touches
//! the disk.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use crate::chunk_set::ChunkSet;
use crate::compress::decode_chunk;
use crate::integrity::{Hash, chunk_hash};
use crate::journal::Journal;
use crate::manifest::{MAX_CHUNK_SIZE, Manifest, ManifestItem, PACK_ITEM_ID};
use crate::messages::{ChunkHeader, ChunkRange};
use crate::pack::{Unpacker, is_packable};
use crate::progress::Progress;
use crate::receiver_store::{STAGING_DIR, Store};
use crate::{BeamError, Result};

/// Receives one transfer into a destination folder.
#[derive(Debug)]
pub struct Receiver {
    manifest: Manifest,
    selected: HashSet<u32>,
    store: Store,
    journal: Journal,
    /// Chunk hashes seen this run, for root checks; earlier runs' chunks are re-read at finish.
    hashes: HashMap<u32, Vec<Option<Hash>>>,
    /// Written but not yet flushed and journaled.
    pending: Vec<(u32, u64)>,
    dirty: HashSet<u32>,
    unpacker: Unpacker,
    pack_next: u64,
    /// Chunks still expected this session. Only a hint for when to checkpoint: a misbehaving
    /// sender can make it reach zero early, which costs one extra checkpoint and nothing else.
    remaining: u64,
    progress: Progress,
}

impl Receiver {
    /// Prepares to receive `manifest` into `destination`, which must exist. `selection` lists
    /// the wanted item IDs; empty means all.
    ///
    /// # Errors
    ///
    /// Returns [`BeamError::Malformed`] for an unknown selected item or a path inside the
    /// staging folder, and a storage error if staging cannot be created.
    pub fn open(manifest: Manifest, destination: &Path, selection: &[u32]) -> Result<Self> {
        let selected: HashSet<u32> = if selection.is_empty() {
            manifest.items().iter().map(|i| i.item_id).collect()
        } else {
            selection.iter().copied().collect()
        };
        let known: HashSet<u32> = manifest.items().iter().map(|i| i.item_id).collect();
        if !selected.is_subset(&known) {
            return Err(BeamError::Malformed(
                "selection names an unknown item".into(),
            ));
        }
        if manifest
            .items()
            .iter()
            .any(|i| i.path.segments()[0].eq_ignore_ascii_case(STAGING_DIR))
        {
            return Err(BeamError::Malformed(
                "path uses the reserved staging folder".into(),
            ));
        }
        let store = Store::open(destination, manifest.transfer_id())?;
        let journal = Journal::open(&store.journal_path(), &manifest)?;
        let mut hashes = HashMap::new();
        for item in manifest.items() {
            if selected.contains(&item.item_id) && !is_packable(item) {
                let count = usize::try_from(item.layout()?.chunk_count())
                    .map_err(|_| BeamError::Malformed("too many chunks".into()))?;
                hashes.insert(item.item_id, vec![None; count]);
            }
        }
        let unpacker = Unpacker::new(&manifest);
        let mut receiver = Self {
            manifest,
            selected,
            store,
            journal,
            hashes,
            pending: Vec::new(),
            dirty: HashSet::new(),
            unpacker,
            pack_next: 0,
            remaining: 0,
            progress: Progress::default(),
        };
        receiver.begin_session();
        Ok(receiver)
    }

    /// The transfer's manifest.
    #[must_use]
    pub fn manifest(&self) -> &Manifest {
        &self.manifest
    }

    /// True if earlier progress was recovered, so the reply should be `RESUME`.
    #[must_use]
    pub fn resumed(&self) -> bool {
        self.journal.resumed()
    }

    /// Resets per-connection state. The sender restarts the pack stream on every connection,
    /// sending only the small files not yet recorded. Empty files are never sent: they have no
    /// chunks, so they count as received from the start and are created when publishing.
    pub fn begin_session(&mut self) {
        self.pack_next = 0;
        self.unpacker = Unpacker::new(&self.manifest);
        for item in self.manifest.items() {
            let done = self.is_item_done(item.item_id);
            if done || !self.selected.contains(&item.item_id) {
                self.unpacker.skip(item.item_id);
            }
        }
        self.remaining = self
            .manifest
            .items()
            .iter()
            .filter(|item| self.selected.contains(&item.item_id))
            .map(|item| {
                let total = item.layout().map_or(0, |layout| layout.chunk_count());
                let have = self
                    .journal
                    .progress(item.item_id)
                    .map_or(0, ChunkSet::count);
                total.saturating_sub(have)
            })
            .sum();
        self.reset_progress();
    }

    /// Verifies one `CHUNK` frame payload and writes its data to staging.
    ///
    /// # Errors
    ///
    /// Returns [`BeamError::Malformed`] for chunks the manifest does not allow,
    /// [`BeamError::IntegrityMismatch`] for data that fails its hash, and storage errors.
    pub fn handle_chunk(&mut self, payload: &[u8]) -> Result<()> {
        let (header, body) = ChunkHeader::decode(payload)?;
        if header.item_id == PACK_ITEM_ID {
            return self.handle_pack(&header, body);
        }
        let item = self.selected_item(header.item_id)?.clone();
        if is_packable(&item) {
            return Err(BeamError::Malformed(
                "small files must arrive in the pack".into(),
            ));
        }
        let layout = item.layout()?;
        let range = layout.range(header.index)?;
        let len = usize::try_from(range.end - range.start)
            .map_err(|_| BeamError::Malformed("chunk does not fit in memory".into()))?;
        if usize::try_from(header.raw_len).ok() != Some(len) {
            return Err(BeamError::Malformed(
                "chunk length does not match the manifest".into(),
            ));
        }
        let is_last = header.index + 1 == layout.chunk_count();
        if header.root.is_some() != is_last {
            return Err(BeamError::Malformed(
                "root hash must come with the last chunk".into(),
            ));
        }
        let data = decode_chunk(header.encoding, body, len)?;
        layout.verify_chunk(header.index, &data, &header.hash)?;
        if self.is_chunk_done(item.item_id, header.index) {
            return Ok(()); // a resend after a reconnect
        }
        self.store.write_at(item.item_id, range.start, &data)?;
        if let Some(root) = &header.root {
            self.store.save_root(item.item_id, root)?;
        }
        if let Some(slot) = self
            .hashes
            .get_mut(&item.item_id)
            .and_then(|h| h.get_mut(usize::try_from(header.index).ok()?))
        {
            *slot = Some(header.hash);
        }
        self.pending.push((item.item_id, header.index));
        self.progress.add(range.end - range.start);
        self.remaining = self.remaining.saturating_sub(1);
        self.dirty.insert(item.item_id);
        Ok(())
    }

    fn handle_pack(&mut self, header: &ChunkHeader, body: &[u8]) -> Result<()> {
        if header.index != self.pack_next || header.root.is_some() {
            return Err(BeamError::Malformed(
                "pack pieces arrived out of order".into(),
            ));
        }
        if header.raw_len > MAX_CHUNK_SIZE {
            return Err(BeamError::Malformed("pack piece is too large".into()));
        }
        let data = decode_chunk(header.encoding, body, header.raw_len as usize)?;
        if chunk_hash(&data) != header.hash {
            return Err(BeamError::IntegrityMismatch {
                chunk: header.index,
            });
        }
        self.pack_next += 1;
        for file in self.unpacker.feed(&data)? {
            self.store.write_whole(file.item_id, &file.data)?;
            if !file.data.is_empty() {
                self.pending.push((file.item_id, 0));
                self.progress.add(file.data.len() as u64);
                self.remaining = self.remaining.saturating_sub(1);
            }
            self.dirty.insert(file.item_id);
        }
        Ok(())
    }

    /// Flushes written data, records it in the journal, and returns the newly durable ranges to
    /// acknowledge.
    ///
    /// # Errors
    ///
    /// Returns a storage error if flushing or journaling fails.
    pub fn checkpoint(&mut self) -> Result<Vec<ChunkRange>> {
        for item_id in std::mem::take(&mut self.dirty) {
            self.store.sync(item_id)?;
        }
        let mut done = std::mem::take(&mut self.pending);
        if done.is_empty() {
            return Ok(Vec::new());
        }
        self.journal.record(&done)?;
        done.sort_unstable();
        done.dedup();
        for &(item_id, _) in &done {
            if self.is_item_done(item_id) {
                self.store.close(item_id);
            }
        }
        Ok(coalesce(&done))
    }

    /// True once every selected item is on disk and journaled.
    #[must_use]
    pub fn is_complete(&self) -> bool {
        self.pending.is_empty() && self.selected.iter().all(|&id| self.is_item_done(id))
    }

    fn selected_item(&self, item_id: u32) -> Result<&ManifestItem> {
        self.manifest
            .item(item_id)
            .filter(|_| self.selected.contains(&item_id))
            .ok_or_else(|| BeamError::Malformed(format!("item {item_id} was not accepted")))
    }

    fn is_item_done(&self, item_id: u32) -> bool {
        self.journal
            .progress(item_id)
            .is_some_and(ChunkSet::is_complete)
    }

    fn is_chunk_done(&self, item_id: u32, index: u64) -> bool {
        self.journal
            .progress(item_id)
            .is_some_and(|set| set.contains(index))
    }
}

#[path = "receiver_finish.rs"]
mod finish;
use finish::coalesce;

#[cfg(test)]
#[path = "receiver_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "receiver_edge_tests.rs"]
mod edge_tests;
