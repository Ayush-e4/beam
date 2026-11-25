//! Opening a [`Receiver`]: checking the offer and finding its staging folder.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use super::Receiver;
use crate::journal::Journal;
use crate::manifest::Manifest;
use crate::pack::{Unpacker, is_packable};
use crate::progress::Progress;
use crate::receiver_store::{STAGING_DIR, Store};
use crate::{BeamError, Result, staging};

impl Receiver {
    /// Prepares to receive `manifest` into `destination`, from a sender whose key isn't known.
    /// See [`Receiver::open_from`].
    ///
    /// # Errors
    ///
    /// As for [`Receiver::open_from`].
    pub fn open(manifest: Manifest, destination: &Path, selection: &[u32]) -> Result<Self> {
        Self::open_from(manifest, destination, selection, &[])
    }

    /// Prepares to receive `manifest` into `destination`, which must exist, from the device
    /// with public key `sender`. `selection` lists the wanted item IDs; empty means all. It
    /// resumes earlier progress of this transfer from the same device; another device's
    /// progress is never mixed in.
    ///
    /// # Errors
    ///
    /// Returns [`BeamError::Malformed`] for an unknown selected item or a path inside the
    /// staging folder, and a storage error if staging cannot be created.
    pub fn open_from(
        manifest: Manifest,
        destination: &Path,
        selection: &[u32],
        sender: &[u8],
    ) -> Result<Self> {
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
        let folder = staging::claim(destination, manifest.transfer_id(), sender)?;
        let store = Store::open(destination, folder);
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
}
