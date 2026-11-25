//! M1 exit: transfers over TCP that keep getting cut, stalled and reconnected must still finish
//! byte-identical, resuming from the journal each time and never duplicating a file.

// Helpers here are test code too; a failed unwrap is how they report a broken setup.
#![allow(clippy::unwrap_used, clippy::panic)]

#[allow(dead_code)] // each test binary uses a different part of the shared helpers
mod common;

use std::net::{Ipv4Addr, SocketAddr};
use std::ops::Range;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use beam_core::{Cipher, Connection, Handshake, Sender, StaticKeypair};
use common::fault::{Faulty, Rng};
use common::{Folders, MIB, assert_same_tree, deliver};
use tokio::net::{TcpListener, TcpStream};

type Link = Connection<Faulty<TcpStream>>;

/// Two trusted devices and a listener; every call to [`Lab::connect`] makes a new link where
/// one side (usually the sender) will be cut after a random number of bytes.
struct Lab {
    laptop: StaticKeypair,
    phone: StaticKeypair,
    listener: TcpListener,
    addr: SocketAddr,
    rng: Rng,
    cuts: Cuts,
    sent: Arc<AtomicU64>,
}

/// Where links get cut: after `sender` bytes of data (on the sender's writes or the receiver's
/// reads), or after `receiver` bytes of the receiver's replies and acks. Ranges are sized so a
/// cut usually lands mid-transfer.
#[derive(Clone)]
struct Cuts {
    sender: Range<u64>,
    receiver: Range<u64>,
}

impl Lab {
    async fn new(seed: u64, cuts: Cuts) -> Self {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        Self {
            laptop: StaticKeypair::generate().unwrap(),
            phone: StaticKeypair::generate().unwrap(),
            addr: listener.local_addr().unwrap(),
            listener,
            rng: Rng::new(seed),
            cuts,
            sent: Arc::new(AtomicU64::new(0)),
        }
    }

    async fn connect(&mut self) -> Option<(Link, Link)> {
        // Cut the sender while it writes, the receiver while it reads (it notices the link died
        // first), or the receiver while it writes acks or `done`.
        let roll = self.rng.between(0..100);
        let data_cut = self.rng.between(self.cuts.sender.clone());
        let (sender_budget, receiver_reads, receiver_budget) = match roll {
            0..50 => (data_cut, u64::MAX, u64::MAX),
            50..80 => (u64::MAX, data_cut, u64::MAX),
            _ => (
                u64::MAX,
                u64::MAX,
                self.rng.between(self.cuts.receiver.clone()),
            ),
        };
        let (client, server) = tokio::join!(TcpStream::connect(self.addr), self.listener.accept());
        let (client, server) = (client.ok()?, server.ok()?.0);
        let unused = Arc::new(AtomicU64::new(0));
        let a = Faulty::new(
            client,
            sender_budget,
            Rng::new(self.rng.next()),
            self.sent.clone(),
        );
        let b = Faulty::new(server, receiver_budget, Rng::new(self.rng.next()), unused)
            .failing_reads_after(receiver_reads);
        let init = Handshake::known_initiator(&self.laptop, Cipher::AesGcm, self.phone.public());
        let resp = Handshake::known_responder(&self.phone, Cipher::AesGcm);
        let (a, b) = tokio::join!(
            Connection::establish(a, init.ok()?),
            Connection::establish(b, resp.ok()?)
        );
        Some((a.ok()?, b.ok()?))
    }
}

async fn survive(folders: &Folders, seed: u64, cuts: Cuts, selection: &[u32]) {
    survive_with(folders, &folders.sender(), seed, cuts, selection).await;
}

async fn survive_with(
    folders: &Folders,
    sender: &Arc<Mutex<Sender>>,
    seed: u64,
    cuts: Cuts,
    selection: &[u32],
) {
    let total: u64 = walk_size(&folders.source);
    let mut lab = Lab::new(seed, cuts).await;
    let report = deliver(sender, &folders.dest, selection, 400, async || {
        lab.connect().await
    })
    .await;
    let sent = lab.sent.load(Ordering::Relaxed);
    eprintln!(
        "seed {seed}: {} attempts, {sent} bytes sent for {total}",
        report.attempts
    );
    assert!(report.attempts >= 5, "the link was cut repeatedly");
    // Every cut is smaller than the whole transfer, so finishing at all proves each attempt
    // resumed from the journal. Bytes sent also include the offer repeated on every link.
    assert!(sent < total * 4, "resumes reuse earlier progress");
}

fn walk_size(dir: &std::path::Path) -> u64 {
    std::fs::read_dir(dir)
        .unwrap()
        .map(|e| {
            let e = e.unwrap();
            if e.file_type().unwrap().is_dir() {
                walk_size(&e.path())
            } else {
                e.metadata().unwrap().len()
            }
        })
        .sum()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_large_file_survives_repeated_cuts() {
    let f = Folders::new();
    f.file("video.bin", 48 * MIB + 12_345, false, 1);
    f.file("notes.txt", 9 * MIB, true, 2);
    // Most cuts exceed one 4 MiB chunk, so every link moves some of the large file.
    let cuts = Cuts {
        sender: 3 * MIB as u64..10 * MIB as u64,
        receiver: 100..600,
    };
    survive(&f, 0x5eed_0001, cuts, &[]).await;
    assert_same_tree(&f.source, &f.dest.join("send"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn ten_thousand_small_files_survive_repeated_cuts() {
    let f = Folders::new();
    f.small_files(10_000);
    f.file("small/empty.dat", 0, false, 0);
    // The offer for 10,000 files is about 800 KB and the receiver's acks about 200 KB.
    let cuts = Cuts {
        sender: 2 * MIB as u64..6 * MIB as u64,
        receiver: 2_000..40_000,
    };
    survive(&f, 0x5eed_0002, cuts, &[]).await;
    assert_same_tree(&f.source, &f.dest.join("send"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "1 GiB: run in release with `cargo test --release -- --ignored`"]
async fn a_one_gib_file_survives_repeated_cuts() {
    let f = Folders::new();
    f.file("disk.img", 1024 * MIB, false, 5);
    let cuts = Cuts {
        sender: 32 * MIB as u64..256 * MIB as u64,
        receiver: 150..6_000,
    };
    survive(&f, 0x5eed_0005, cuts, &[]).await;
    assert_same_tree(&f.source, &f.dest.join("send"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_selection_survives_repeated_cuts() {
    let f = Folders::new();
    f.file("big.bin", 20 * MIB, false, 7);
    f.small_files(600);
    let sender = f.sender();
    let manifest = sender.lock().unwrap().manifest().clone();
    let picked: Vec<u32> = (0..600).step_by(3).collect();
    // Cuts must exceed one 4 MiB chunk, or the large file could never get a chunk through.
    let cuts = Cuts {
        sender: 5 * MIB as u64..10 * MIB as u64,
        receiver: 100..6_000,
    };
    survive_with(&f, &sender, 0x5eed_0007, cuts, &picked).await;

    for item in manifest.items() {
        let source = f.source.parent().unwrap().join(item.path.to_string());
        let received = item.path.resolve_in(&f.dest);
        if picked.contains(&item.item_id) {
            assert_eq!(
                std::fs::read(&source).unwrap(),
                std::fs::read(&received).unwrap()
            );
        } else {
            assert!(!received.exists(), "{} was not picked", item.path);
        }
    }
}
