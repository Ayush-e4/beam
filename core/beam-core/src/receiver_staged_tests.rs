//! A receiver staging into memory instead of a folder, as the browser stages into OPFS: whole
//! transfers and resumes work through [`TransferStaging`] alone, with nothing on disk.

use std::io;
use std::ops::Range;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use super::tests::{chunks_for, fixture, pack_piece};
use super::*;
use crate::storage_file::StorageFile;

/// What the memory staging holds; it outlives a receiver, like files survive an app restart.
#[derive(Debug, Default)]
struct Shelf {
    parts: HashMap<u32, Vec<u8>>,
    roots: HashMap<u32, Hash>,
    journal: Vec<u8>,
    published: HashMap<PathBuf, Vec<u8>>,
    removed: bool,
}

type Shared = Arc<Mutex<Shelf>>;

#[derive(Debug)]
struct MemoryStaging(Shared);

#[derive(Debug)]
struct MemoryJournal(Shared);

fn write_into(bytes: &mut Vec<u8>, data: &[u8], offset: u64) {
    let start = usize::try_from(offset).unwrap();
    if bytes.len() < start + data.len() {
        bytes.resize(start + data.len(), 0);
    }
    bytes[start..start + data.len()].copy_from_slice(data);
}

impl StorageFile for MemoryJournal {
    fn size(&mut self) -> io::Result<u64> {
        Ok(self.0.lock().unwrap().journal.len() as u64)
    }

    fn read_exact_at(&mut self, buf: &mut [u8], offset: u64) -> io::Result<()> {
        let held = self.0.lock().unwrap();
        let start = usize::try_from(offset).unwrap();
        let bytes = held.journal.get(start..start + buf.len());
        buf.copy_from_slice(bytes.ok_or(io::ErrorKind::UnexpectedEof)?);
        Ok(())
    }

    fn write_all_at(&mut self, data: &[u8], offset: u64) -> io::Result<()> {
        write_into(&mut self.0.lock().unwrap().journal, data, offset);
        Ok(())
    }

    fn set_len(&mut self, len: u64) -> io::Result<()> {
        let len = usize::try_from(len).unwrap();
        self.0.lock().unwrap().journal.resize(len, 0);
        Ok(())
    }

    fn sync(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl TransferStaging for MemoryStaging {
    fn journal(&mut self) -> Result<Box<dyn StorageFile>> {
        Ok(Box::new(MemoryJournal(Arc::clone(&self.0))))
    }

    fn write_at(&mut self, item_id: u32, offset: u64, data: &[u8]) -> Result<()> {
        let mut held = self.0.lock().unwrap();
        write_into(held.parts.entry(item_id).or_default(), data, offset);
        Ok(())
    }

    fn write_whole(&mut self, item_id: u32, data: &[u8]) -> Result<()> {
        self.0.lock().unwrap().parts.insert(item_id, data.to_vec());
        Ok(())
    }

    fn sync(&mut self, _item_id: u32) -> Result<()> {
        Ok(())
    }

    fn close(&mut self, _item_id: u32) {}

    fn read_range(&mut self, item_id: u32, range: Range<u64>) -> Result<Vec<u8>> {
        let held = self.0.lock().unwrap();
        let part = held.parts.get(&item_id).map_or(&[][..], Vec::as_slice);
        let range = usize::try_from(range.start).unwrap()..usize::try_from(range.end).unwrap();
        part.get(range)
            .map(<[u8]>::to_vec)
            .ok_or_else(|| BeamError::storage(io::ErrorKind::UnexpectedEof.into()))
    }

    fn save_root(&mut self, item_id: u32, root: &Hash) -> Result<()> {
        self.0.lock().unwrap().roots.insert(item_id, *root);
        Ok(())
    }

    fn load_root(&mut self, item_id: u32) -> Result<Hash> {
        let root = self.0.lock().unwrap().roots.get(&item_id).copied();
        root.ok_or_else(|| BeamError::storage(io::ErrorKind::NotFound.into()))
    }

    fn publish(&mut self, item: &ManifestItem) -> Result<PathBuf> {
        let mut held = self.0.lock().unwrap();
        let data = held.parts.remove(&item.item_id).unwrap_or_default();
        let path = PathBuf::from(item.path.to_string());
        held.published.insert(path.clone(), data);
        Ok(path)
    }

    fn remove(self: Box<Self>) -> Result<()> {
        self.0.lock().unwrap().removed = true;
        Ok(())
    }
}

fn open(f: &super::tests::Fixture, held: &Shared) -> Receiver {
    let staging = Box::new(MemoryStaging(Arc::clone(held)));
    Receiver::open_staged(f.manifest.clone(), &[], staging).unwrap()
}

fn assert_published(f: &super::tests::Fixture, shelf: &Shared) {
    let shelf = shelf.lock().unwrap();
    for item in f.manifest.items() {
        let path = PathBuf::from(item.path.to_string());
        assert_eq!(
            shelf.published.get(&path),
            Some(&f.contents[&item.item_id]),
            "{path:?}"
        );
    }
    assert!(shelf.removed, "staging is cleaned up");
}

#[test]
fn a_whole_transfer_lands_in_memory_staging() {
    let (f, shelf) = (fixture(), Shared::default());
    let mut r = open(&f, &shelf);
    assert!(!r.resumed());
    for id in [1, 2] {
        for payload in chunks_for(&f, id) {
            r.handle_chunk(&payload).unwrap();
        }
    }
    r.handle_chunk(&pack_piece(&f, 0, &[3, 5])).unwrap();
    r.checkpoint().unwrap();
    assert!(r.is_complete());
    assert_eq!(r.finish().unwrap().len(), 5);
    assert_published(&f, &shelf);
}

#[test]
fn an_interrupted_transfer_resumes_from_the_staged_journal() {
    let (f, shelf) = (fixture(), Shared::default());
    let big = chunks_for(&f, 1);
    {
        let mut r = open(&f, &shelf);
        r.handle_chunk(&big[0]).unwrap();
        r.handle_chunk(&big[1]).unwrap();
        r.checkpoint().unwrap();
        r.handle_chunk(&big[2]).unwrap(); // written, never acknowledged: the tab closes here
    }
    let mut r = open(&f, &shelf);
    assert!(r.resumed());
    let have = r.have();
    assert!(have.contains(&ChunkRange {
        item_id: 1,
        chunks: 0..2
    }));
    assert!(!have.iter().any(|h| h.item_id == 1 && h.chunks.contains(&2)));

    for payload in &big[2..] {
        r.handle_chunk(payload).unwrap();
    }
    for payload in chunks_for(&f, 2) {
        r.handle_chunk(&payload).unwrap();
    }
    r.handle_chunk(&pack_piece(&f, 0, &[3, 5])).unwrap();
    r.finish().unwrap();
    assert_published(&f, &shelf);
}

#[test]
fn a_torn_journal_record_is_cut_off_on_reopen() {
    let (f, shelf) = (fixture(), Shared::default());
    {
        let mut r = open(&f, &shelf);
        r.handle_chunk(&chunks_for(&f, 1)[0]).unwrap();
        r.checkpoint().unwrap();
    }
    let good_len = shelf.lock().unwrap().journal.len();
    shelf.lock().unwrap().journal.extend_from_slice(&[0xAB; 7]); // a crash mid-append

    let r = open(&f, &shelf);
    assert!(r.resumed());
    assert_eq!(
        shelf.lock().unwrap().journal.len(),
        good_len,
        "the torn tail is cut off"
    );
}
