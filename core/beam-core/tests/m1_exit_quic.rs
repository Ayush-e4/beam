//! M1 exit: transfers over QUIC through a lossy, reordering link that keeps dying must still
//! finish byte-identical, resuming from the journal each time and never duplicating a file.

// Helpers here are test code too; a failed unwrap is how they report a broken setup.
#![allow(clippy::unwrap_used, clippy::panic)]

#[allow(dead_code)] // each test binary uses a different part of the shared helpers
mod common;

use std::net::Ipv4Addr;
use std::sync::Arc;

use beam_core::quic::{QuicEndpoint, QuicStream};
use beam_core::{Cipher, Connection, Handshake, StaticKeypair};
use common::fault::Rng;
use common::udp_proxy::{LinkProfile, Proxy};
use common::{Folders, MIB, assert_same_tree, deliver};

type Link = Connection<QuicStream>;

/// Two trusted devices; every call to [`Lab::connect`] builds fresh endpoints and a lossy relay
/// that dies after a random number of bytes. When it dies both endpoints are closed, which is
/// how the apps will react to a lost network, so neither side waits for QUIC's idle timeout.
struct Lab {
    laptop: StaticKeypair,
    phone: StaticKeypair,
    rng: Rng,
    loss_percent: std::ops::Range<u64>,
    budget: std::ops::Range<u64>,
    proxy: Option<Proxy>,
}

impl Lab {
    fn new(seed: u64, loss_percent: std::ops::Range<u64>, budget: std::ops::Range<u64>) -> Self {
        Self {
            laptop: StaticKeypair::generate().unwrap(),
            phone: StaticKeypair::generate().unwrap(),
            rng: Rng::new(seed),
            loss_percent,
            budget,
            proxy: None,
        }
    }

    async fn connect(&mut self) -> Option<(Link, Link)> {
        self.proxy = None; // the previous link, if any, is gone
        let client = Arc::new(QuicEndpoint::bind((Ipv4Addr::LOCALHOST, 0).into()).unwrap());
        let server = Arc::new(QuicEndpoint::bind((Ipv4Addr::LOCALHOST, 0).into()).unwrap());
        let profile = LinkProfile {
            loss_percent: self.rng.between(self.loss_percent.clone()),
            reorder_percent: 2,
            budget: self.rng.between(self.budget.clone()),
        };
        let (cut_client, cut_server) = (Arc::clone(&client), Arc::clone(&server));
        let on_cut = move || {
            cut_client.close();
            cut_server.close();
        };
        let server_addr = server.local_addr().unwrap();
        let proxy = Proxy::start(server_addr, profile, Rng::new(self.rng.next()), on_cut).await;
        let init = Handshake::known_initiator(&self.laptop, Cipher::AesGcm, self.phone.public());
        let resp = Handshake::known_responder(&self.phone, Cipher::AesGcm);
        let (a, b) = tokio::join!(
            client.connect(proxy.addr, init.ok()?),
            server.accept(resp.ok()?)
        );
        self.proxy = Some(proxy);
        Some((a.ok()?, b.ok()?.0))
    }
}

async fn survive(f: &Folders, lab: &mut Lab) {
    let sender = f.sender();
    let report = deliver(&sender, &f.dest, &[], 400, async || lab.connect().await).await;
    eprintln!("{} attempts", report.attempts);
    assert!(report.attempts > 3, "the link was cut repeatedly");
    assert_same_tree(&f.source, &f.dest.join("send"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_large_file_survives_loss_reordering_and_cuts() {
    let f = Folders::new();
    f.file("video.bin", 24 * MIB + 777, false, 3);
    f.file("notes.txt", 6 * MIB, true, 4);
    let mut lab = Lab::new(0x5eed_0003, 1..6, 2 * MIB as u64..10 * MIB as u64);
    survive(&f, &mut lab).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn ten_thousand_small_files_survive_loss_reordering_and_cuts() {
    let f = Folders::new();
    f.small_files(10_000);
    let mut lab = Lab::new(0x5eed_0004, 1..6, MIB as u64..6 * MIB as u64);
    survive(&f, &mut lab).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "1 GiB: run in release with `cargo test --release -- --ignored`"]
async fn a_one_gib_file_survives_loss_reordering_and_cuts() {
    let f = Folders::new();
    f.file("disk.img", 1024 * MIB, false, 6);
    let mut lab = Lab::new(0x5eed_0006, 1..6, 64 * MIB as u64..320 * MIB as u64);
    survive(&f, &mut lab).await;
}
