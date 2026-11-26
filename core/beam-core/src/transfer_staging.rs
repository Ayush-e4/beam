//! Where an incoming transfer's data waits until every file is verified, behind one trait so
//! the receiver doesn't care what holds it: a staging folder next to the destination on the
//! phone and the Mac ([`crate::receiver_store`]), browser storage (OPFS) in the web app.

use std::fmt;
use std::ops::Range;
use std::path::PathBuf;

use crate::Result;
use crate::integrity::Hash;
use crate::manifest::ManifestItem;
use crate::read_at::MaybeSend;
use crate::storage_file::StorageFile;

/// One transfer's staged data: a part per item, its root hash, and the resume journal.
pub trait TransferStaging: MaybeSend + fmt::Debug {
    /// The resume journal's file, empty if the transfer is new.
    ///
    /// # Errors
    ///
    /// A storage error if it can't be opened.
    fn journal(&mut self) -> Result<Box<dyn StorageFile>>;

    /// Writes `data` at `offset` in the item's part.
    ///
    /// # Errors
    ///
    /// A storage error if it can't be written.
    fn write_at(&mut self, item_id: u32, offset: u64, data: &[u8]) -> Result<()>;

    /// Replaces the item's part with `data` (a packed small file).
    ///
    /// # Errors
    ///
    /// A storage error if it can't be written.
    fn write_whole(&mut self, item_id: u32, data: &[u8]) -> Result<()>;

    /// Makes the item's written data durable. The journal records chunks only after this.
    ///
    /// # Errors
    ///
    /// A storage error if it can't be flushed.
    fn sync(&mut self, item_id: u32) -> Result<()>;

    /// The item needs no more writes, so anything held open for it can be let go.
    fn close(&mut self, item_id: u32);

    /// Reads back `range` of the item's part.
    ///
    /// # Errors
    ///
    /// A storage error if it can't be read.
    fn read_range(&mut self, item_id: u32, range: Range<u64>) -> Result<Vec<u8>>;

    /// Durably stores the item's root hash.
    ///
    /// # Errors
    ///
    /// A storage error if it can't be stored.
    fn save_root(&mut self, item_id: u32, root: &Hash) -> Result<()>;

    /// The item's stored root hash.
    ///
    /// # Errors
    ///
    /// A storage error if it's missing, [`crate::BeamError::Malformed`] if it's damaged.
    fn load_root(&mut self, item_id: u32) -> Result<Hash>;

    /// Hands over a verified item, returning where it ended up.
    ///
    /// # Errors
    ///
    /// A storage error if it can't be moved into place.
    fn publish(&mut self, item: &ManifestItem) -> Result<PathBuf>;

    /// Removes what is left once every item is published.
    ///
    /// # Errors
    ///
    /// A storage error if it can't be removed.
    fn remove(self: Box<Self>) -> Result<()>;
}
