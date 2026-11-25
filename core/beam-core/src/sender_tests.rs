//! The sender against the real receiver, in-process, with real files on disk.

use std::fs;
use std::path::{Path, PathBuf};

use tempfile::TempDir;

use super::*;
use crate::Receiver;
use crate::integrity::chunk_hash;
use crate::messages::ChunkHeader;
use crate::sender_files::{DEFAULT_CHUNK_SIZE, prepare_transfer_with_id};

const ID: TransferId = TransferId([9; 16]);
const CHUNK: usize = DEFAULT_CHUNK_SIZE as usize;

struct Setup {
    _dir: TempDir,
    source: PathBuf,
    dest: PathBuf,
    sender: Sender,
}

/// Item IDs follow name order: 0 big.txt, 1 empty.txt, 2 medium.bin, 3 noise.bin, 4.. small/*.
fn setup(window: u64) -> Setup {
    let dir = TempDir::new().unwrap();
    let source = dir.path().join("album");
    let dest = dir.path().join("dest");
    fs::create_dir_all(source.join("small")).unwrap();
    fs::create_dir_all(&dest).unwrap();
    let text: Vec<u8> = b"beam moves files ".repeat(2 * CHUNK / 17 + 100);
    fs::write(source.join("big.txt"), &text).unwrap();
    fs::write(source.join("empty.txt"), b"").unwrap();
    fs::write(source.join("medium.bin"), noise(300 * 1024, 1)).unwrap();
    fs::write(source.join("noise.bin"), noise(CHUNK + 10, 2)).unwrap();
    for n in 0..300 {
        fs::write(source.join(format!("small/{n:03}.txt")), noise(5000, n)).unwrap();
    }
    let prepared = prepare_transfer_with_id(std::slice::from_ref(&source), ID).unwrap();
    Setup {
        _dir: dir,
        source,
        dest,
        sender: Sender::with_window(prepared, window),
    }
}

fn noise(len: usize, seed: u64) -> Vec<u8> {
    let mut out = Vec::with_capacity(len + 32);
    let mut counter = 0u64;
    while out.len() < len {
        out.extend_from_slice(&chunk_hash(
            &[seed.to_le_bytes(), counter.to_le_bytes()].concat(),
        ));
        counter += 1;
    }
    out.truncate(len);
    out
}

/// Runs the connection until nothing more moves. `deliver` decides which payloads reach the
/// receiver (dropping some simulates a connection that dies). Returns the payloads sent.
fn pump(s: &mut Sender, r: &mut Receiver, mut deliver: impl FnMut(&ChunkHeader) -> bool) -> usize {
    let mut sent = 0;
    loop {
        while s.can_send() {
            let Some(payload) = s.next_chunk().unwrap() else {
                break;
            };
            sent += 1;
            assert!(s.in_flight() <= s.window + u64::from(DEFAULT_CHUNK_SIZE));
            if deliver(&ChunkHeader::decode(&payload).unwrap().0) {
                r.handle_chunk(&payload).unwrap();
            }
        }
        let acks = r.checkpoint().unwrap();
        if acks.is_empty() {
            return sent;
        }
        s.handle_ack(&acks);
    }
}

fn accept_all() -> Accept {
    Accept {
        transfer_id: ID,
        items: Vec::new(),
    }
}

fn assert_same_tree(source: &Path, received: &Path) {
    let mut names: Vec<_> = fs::read_dir(source).unwrap().map(|e| e.unwrap()).collect();
    names.sort_by_key(fs::DirEntry::file_name);
    for entry in names {
        let there = received.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            assert_same_tree(&entry.path(), &there);
        } else {
            assert_eq!(fs::read(entry.path()).unwrap(), fs::read(&there).unwrap());
        }
    }
}

#[test]
fn a_whole_folder_arrives_identical() {
    let mut t = setup(DEFAULT_WINDOW);
    let mut r = Receiver::open(t.sender.manifest().clone(), &t.dest, &[]).unwrap();
    t.sender.accept(&accept_all()).unwrap();

    let mut compressed = false;
    pump(&mut t.sender, &mut r, |h| {
        compressed |= h.encoding == crate::compress::ChunkEncoding::Zstd;
        true
    });
    assert!(t.sender.is_complete());
    assert!(compressed, "the text file is compressed");
    assert!(r.is_complete());
    r.finish().unwrap();
    assert_same_tree(&t.source, &t.dest.join("album"));
}

#[test]
fn only_the_accepted_items_are_sent() {
    let mut t = setup(DEFAULT_WINDOW);
    let wanted = vec![0, 4, 5];
    let mut r = Receiver::open(t.sender.manifest().clone(), &t.dest, &wanted).unwrap();
    t.sender
        .accept(&Accept {
            transfer_id: ID,
            items: wanted,
        })
        .unwrap();
    pump(&mut t.sender, &mut r, |_| true);
    assert!(t.sender.is_complete());
    r.finish().unwrap();
    let album = t.dest.join("album");
    assert_eq!(fs::read_dir(album.join("small")).unwrap().count(), 2);
    assert!(album.join("big.txt").exists());
    assert!(!album.join("noise.bin").exists());
}

/// Session one delivers only what `first` lets through, then the receiver "crashes" and is
/// reopened from its journal; session two resumes and must finish with identical files.
fn resume_after_crash(first: impl FnMut(&ChunkHeader) -> bool) -> (usize, usize) {
    let mut t = setup(DEFAULT_WINDOW);
    let manifest = t.sender.manifest().clone();
    let mut r = Receiver::open(manifest.clone(), &t.dest, &[]).unwrap();
    t.sender.accept(&accept_all()).unwrap();
    let first_sent = pump(&mut t.sender, &mut r, first);
    assert!(!t.sender.is_complete());
    drop(r);

    let mut r = Receiver::open(manifest, &t.dest, &[]).unwrap();
    assert!(r.resumed());
    t.sender.resume(ID, &r.have()).unwrap();
    let second_sent = pump(&mut t.sender, &mut r, |_| true);
    assert!(t.sender.is_complete());
    r.finish().unwrap();
    assert_same_tree(&t.source, &t.dest.join("album"));
    (first_sent, second_sent)
}

#[test]
fn a_resume_sends_only_what_is_missing() {
    let mut delivered = 0;
    let (first, second) = resume_after_crash(|_| {
        delivered += 1;
        delivered <= 3 // both pack pieces and the first chunk of big.txt
    });
    assert!(second < first, "resend {second} of {first}");
}

#[test]
fn a_resume_works_when_the_receiver_already_has_the_last_chunk() {
    // The receiver misses only big.txt's first chunk, so the root hash it saved earlier is
    // reused and the sender must not re-read the rest of the file.
    let (_, second) = resume_after_crash(|h| !(h.item_id == 0 && h.index == 0));
    // The missing chunk, plus a pack piece holding the empty file: empty files carry no data to
    // journal, so they ride in the pack on every connection.
    assert_eq!(second, 2);
}

#[test]
fn the_window_limits_unacknowledged_bytes() {
    let mut t = setup(1);
    let mut r = Receiver::open(t.sender.manifest().clone(), &t.dest, &[]).unwrap();
    t.sender.accept(&accept_all()).unwrap();
    assert!(t.sender.next_chunk().unwrap().is_some());
    assert!(!t.sender.can_send(), "one chunk fills a 1-byte window");
    let in_flight = t.sender.in_flight();
    assert!(in_flight > 1_000_000, "the first pack piece is about 1 MiB");

    assert_eq!(
        pump(&mut t.sender, &mut r, |_| true),
        0,
        "no room, nothing sent"
    );
    let stranger = ChunkRange {
        item_id: 10_000,
        chunks: 0..1,
    };
    t.sender.handle_ack(&[stranger]);
    assert_eq!(
        t.sender.in_flight(),
        in_flight,
        "unknown acks change nothing"
    );
}

#[test]
fn a_file_changed_after_the_offer_fails_clearly() {
    let mut t = setup(DEFAULT_WINDOW);
    fs::write(t.source.join("big.txt"), b"rewritten").unwrap();
    t.sender.accept(&accept_all()).unwrap();
    let err = loop {
        match t.sender.next_chunk() {
            Ok(Some(_)) => {}
            Ok(None) => panic!("the change went unnoticed"),
            Err(err) => break err,
        }
    };
    assert!(matches!(err, BeamError::SourceChanged { .. }), "{err}");
}

#[test]
fn bad_replies_are_rejected() {
    let mut t = setup(DEFAULT_WINDOW);
    assert!(
        t.sender.next_chunk().is_err(),
        "nothing to send before a reply"
    );
    let other = Accept {
        transfer_id: TransferId([1; 16]),
        items: Vec::new(),
    };
    assert!(t.sender.accept(&other).is_err());
    let unknown = Accept {
        transfer_id: ID,
        items: vec![10_000],
    };
    assert!(t.sender.accept(&unknown).is_err());
    let past_end = [ChunkRange {
        item_id: 0,
        chunks: 0..9,
    }];
    assert!(t.sender.resume(ID, &past_end).is_err());
    let stranger = [ChunkRange {
        item_id: 10_000,
        chunks: 0..1,
    }];
    assert!(t.sender.resume(ID, &stranger).is_err());
}
