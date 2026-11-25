//! End-to-end transfers: real folders sent between two in-process peers over TCP and QUIC.

// Helpers here are test code too; a failed unwrap is how they report a broken setup.
#![allow(clippy::unwrap_used)]

use std::fs;
use std::net::Ipv4Addr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use beam_core::messages::Reason;
use beam_core::quic::QuicEndpoint;
use beam_core::tcp::{self, TcpAcceptor};
use beam_core::{
    BeamError, Cipher, Connection, Handshake, Receiver, SendOutcome, Sender, StaticKeypair,
    prepare_transfer, read_offer, receive_transfer, reject_offer, send_transfer,
};
use tempfile::TempDir;
use tokio::io::{AsyncRead, AsyncWrite};

const MIB: usize = 1024 * 1024;

struct Folders {
    _dir: TempDir,
    source: PathBuf,
    dest: PathBuf,
}

/// A 9 MiB compressible file, a 5 MiB incompressible one, 400 small files, an empty file and
/// nested folders.
fn folders() -> Folders {
    let dir = TempDir::new().unwrap();
    let source = dir.path().join("trip");
    let dest = dir.path().join("inbox");
    fs::create_dir_all(source.join("photos/raw")).unwrap();
    fs::create_dir_all(&dest).unwrap();
    fs::write(source.join("notes.txt"), b"beam ".repeat(9 * MIB / 5)).unwrap();
    fs::write(source.join("photos/raw/big.bin"), noise(5 * MIB + 3, 0)).unwrap();
    fs::write(source.join("photos/empty.jpg"), b"").unwrap();
    for n in 0..400 {
        let name = format!("photos/{n:03}.jpg");
        fs::write(source.join(name), noise(1000 + n * 37, n as u64 + 1)).unwrap();
    }
    Folders {
        _dir: dir,
        source,
        dest,
    }
}

fn noise(len: usize, seed: u64) -> Vec<u8> {
    let mut out = vec![0; len];
    let mut hasher = blake3::Hasher::new();
    hasher.update(&seed.to_le_bytes());
    hasher.finalize_xof().fill(&mut out);
    out
}

fn assert_same_tree(source: &Path, received: &Path) {
    for entry in fs::read_dir(source).unwrap() {
        let entry = entry.unwrap();
        let there = received.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            assert_same_tree(&entry.path(), &there);
        } else {
            let (want, got) = (fs::read(entry.path()).unwrap(), fs::read(&there).unwrap());
            assert!(want == got, "{} differs", there.display());
        }
    }
}

fn sender_for(f: &Folders) -> Arc<Mutex<Sender>> {
    let prepared = prepare_transfer(std::slice::from_ref(&f.source)).unwrap();
    Arc::new(Mutex::new(Sender::new(prepared)))
}

/// The receiving app: read the offer, accept everything, receive.
async fn accept_all<S: AsyncRead + AsyncWrite + Unpin>(
    conn: &mut Connection<S>,
    dest: &Path,
) -> beam_core::Result<Vec<PathBuf>> {
    let manifest = read_offer(conn).await?;
    let receiver = Receiver::open(manifest, dest, &[])?;
    receive_transfer(conn, receiver).await
}

fn keys() -> (StaticKeypair, StaticKeypair) {
    (
        StaticKeypair::generate().unwrap(),
        StaticKeypair::generate().unwrap(),
    )
}

async fn tcp_pair() -> (
    Connection<tokio::net::TcpStream>,
    Connection<tokio::net::TcpStream>,
) {
    let (laptop, phone) = keys();
    let acceptor = TcpAcceptor::bind((Ipv4Addr::LOCALHOST, 0).into())
        .await
        .unwrap();
    let addr = acceptor.local_addr().unwrap();
    let i = Handshake::known_initiator(&laptop, Cipher::AesGcm, phone.public()).unwrap();
    let r = Handshake::known_responder(&phone, Cipher::AesGcm).unwrap();
    let (a, b) = tokio::join!(tcp::connect(addr, i), acceptor.accept(r));
    (a.unwrap(), b.unwrap().0)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_folder_arrives_identical_over_tcp() {
    let f = folders();
    let (mut a, mut b) = tcp_pair().await;
    let sender = sender_for(&f);
    let (sent, received) =
        tokio::join!(send_transfer(&mut a, &sender), accept_all(&mut b, &f.dest));
    assert_eq!(sent.unwrap(), SendOutcome::Delivered);
    assert_eq!(received.unwrap().len(), 403);
    assert_same_tree(&f.source, &f.dest.join("trip"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_folder_arrives_identical_over_quic() {
    let f = folders();
    let (laptop, phone) = keys();
    let client = QuicEndpoint::bind((Ipv4Addr::LOCALHOST, 0).into()).unwrap();
    let server = QuicEndpoint::bind((Ipv4Addr::LOCALHOST, 0).into()).unwrap();
    let addr = server.local_addr().unwrap();
    let init = Handshake::known_initiator(&laptop, Cipher::ChaChaPoly, phone.public()).unwrap();
    let resp = Handshake::known_responder(&phone, Cipher::ChaChaPoly).unwrap();
    let (a, b) = tokio::join!(client.connect(addr, init), server.accept(resp));
    let (mut a, (mut b, _)) = (a.unwrap(), b.unwrap());

    let sender = sender_for(&f);
    let (sent, received) =
        tokio::join!(send_transfer(&mut a, &sender), accept_all(&mut b, &f.dest));
    assert_eq!(sent.unwrap(), SendOutcome::Delivered);
    assert_eq!(received.unwrap().len(), 403);
    assert_same_tree(&f.source, &f.dest.join("trip"));
}

#[tokio::test]
async fn a_declined_offer_reaches_the_sender() {
    let f = folders();
    let (mut a, mut b) = tcp_pair().await;
    let sender = sender_for(&f);
    let decline = async {
        let manifest = read_offer(&mut b).await?;
        reject_offer(&mut b, manifest.transfer_id(), Reason::UserDeclined).await
    };
    let (sent, declined) = tokio::join!(send_transfer(&mut a, &sender), decline);
    declined.unwrap();
    assert_eq!(sent.unwrap(), SendOutcome::Rejected(Reason::UserDeclined));
    assert_eq!(fs::read_dir(&f.dest).unwrap().count(), 0, "nothing written");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_file_changed_after_the_offer_cancels_on_both_sides() {
    let f = folders();
    let (mut a, mut b) = tcp_pair().await;
    let sender = sender_for(&f);
    fs::write(f.source.join("notes.txt"), b"rewritten after the offer").unwrap();
    let (sent, received) =
        tokio::join!(send_transfer(&mut a, &sender), accept_all(&mut b, &f.dest));
    assert!(
        matches!(sent, Err(BeamError::SourceChanged { .. })),
        "{sent:?}"
    );
    assert!(
        matches!(received, Err(BeamError::Cancelled)),
        "{received:?}"
    );
}
