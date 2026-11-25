//! Shared helpers for the M1 exit tests: test folders, a reconnecting transfer loop and checks.

pub mod fault;
pub mod udp_proxy;

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use beam_core::{
    BeamError, Connection, ErrorCategory, Receiver, SendOutcome, Sender, prepare_transfer,
    read_offer, receive_transfer,
};
use tempfile::TempDir;
use tokio::io::{AsyncRead, AsyncWrite};

pub const MIB: usize = 1024 * 1024;

/// One attempt that hangs longer than this counts as a lost link.
const ATTEMPT_LIMIT: Duration = Duration::from_secs(60);

pub struct Folders {
    _dir: TempDir,
    pub source: PathBuf,
    pub dest: PathBuf,
}

impl Folders {
    pub fn new() -> Self {
        let dir = TempDir::new().unwrap();
        let source = dir.path().join("send");
        let dest = dir.path().join("inbox");
        fs::create_dir_all(&source).unwrap();
        fs::create_dir_all(&dest).unwrap();
        Self {
            _dir: dir,
            source,
            dest,
        }
    }

    /// Writes `len` bytes of incompressible data (or compressible text) at `relative`,
    /// streaming so even a 1 GiB file never sits in memory.
    pub fn file(&self, relative: &str, len: usize, compressible: bool, seed: u64) {
        let path = self.source.join(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mut out = io::BufWriter::new(fs::File::create(path).unwrap());
        let mut noise = blake3::Hasher::new()
            .update(&seed.to_le_bytes())
            .finalize_xof();
        let mut text = b"beam beams files ".iter().copied().cycle();
        let mut block = vec![0u8; 4 * MIB];
        let mut left = len;
        while left > 0 {
            let piece = &mut block[..left.min(4 * MIB)];
            if compressible {
                for (byte, t) in piece.iter_mut().zip(&mut text) {
                    *byte = t;
                }
            } else {
                noise.fill(piece);
            }
            out.write_all(piece).unwrap();
            left -= piece.len();
        }
        out.flush().unwrap();
    }

    /// `count` small files of 0–4 KiB spread over nested folders.
    pub fn small_files(&self, count: u64) {
        for n in 0..count {
            let len = usize::try_from(n * 97 % 4096).unwrap();
            let name = format!("small/{:02}/{:02}/{n:05}.dat", n % 7, n % 13);
            self.file(&name, len, n % 3 == 0, n);
        }
    }

    pub fn sender(&self) -> Arc<Mutex<Sender>> {
        let prepared = prepare_transfer(std::slice::from_ref(&self.source)).unwrap();
        Arc::new(Mutex::new(Sender::new(prepared)))
    }
}

/// How a transfer that survived faults went.
#[derive(Debug)]
pub struct Report {
    pub attempts: u32,
}

/// Keeps reconnecting until the sender reports the transfer delivered, like the apps will:
/// the same [`Sender`] each time, and a [`Receiver`] reopened from its journal. `connect`
/// makes the next link; `None` means the link died during the handshake.
pub async fn deliver<S1, S2>(
    sender: &Arc<Mutex<Sender>>,
    dest: &Path,
    selection: &[u32],
    max_attempts: u32,
    mut connect: impl AsyncFnMut() -> Option<(Connection<S1>, Connection<S2>)>,
) -> Report
where
    S1: AsyncRead + AsyncWrite + Unpin,
    S2: AsyncRead + AsyncWrite + Unpin,
{
    let mut finished = false;
    for attempt in 1..=max_attempts {
        let Some((mut a, mut b)) = connect().await else {
            continue;
        };
        // Each side drops its connection as soon as it is done, so the peer notices at once.
        let sending = async move { beam_core::send_transfer(&mut a, sender).await };
        let receiving = async move { receive_once(&mut b, dest, selection, finished).await };
        let Ok((sent, received)) =
            tokio::time::timeout(ATTEMPT_LIMIT, async { tokio::join!(sending, receiving) }).await
        else {
            continue;
        };
        match received {
            Ok(()) => finished = true,
            Err(err) => assert_link_failure(&err),
        }
        match sent {
            Ok(SendOutcome::Delivered) => {
                assert!(
                    finished,
                    "the sender saw `done` but the receiver did not finish"
                );
                return Report { attempts: attempt };
            }
            Ok(SendOutcome::Rejected(reason)) => panic!("rejected: {reason:?}"),
            Err(err) => assert_link_failure(&err),
        }
    }
    panic!("not delivered after {max_attempts} attempts");
}

/// The receiving app for one connection. Once the files are published it only confirms.
async fn receive_once<S: AsyncRead + AsyncWrite + Unpin>(
    conn: &mut Connection<S>,
    dest: &Path,
    selection: &[u32],
    finished: bool,
) -> beam_core::Result<()> {
    let manifest = read_offer(conn).await?;
    if finished {
        return beam_core::confirm_received(conn, &manifest).await;
    }
    let receiver = Receiver::open(manifest, dest, selection)?;
    receive_transfer(conn, receiver).await.map(|_| ())
}

/// Injected faults must only ever look like a lost link, never like bad data.
fn assert_link_failure(err: &BeamError) {
    assert_eq!(err.category(), ErrorCategory::Network, "{err}");
}

/// Checks every source file arrived byte-identical, and nothing else did (no duplicates).
pub fn assert_same_tree(source: &Path, received: &Path) {
    let mut want: Vec<_> = fs::read_dir(source)
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    let mut got: Vec<_> = fs::read_dir(received)
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    want.sort();
    got.sort();
    assert_eq!(want, got, "entries of {}", received.display());
    for name in want {
        let (from, to) = (source.join(&name), received.join(&name));
        if from.is_dir() {
            assert_same_tree(&from, &to);
        } else {
            assert_eq!(digest(&from), digest(&to), "{} differs", to.display());
        }
    }
}

fn digest(path: &Path) -> blake3::Hash {
    let mut hasher = blake3::Hasher::new();
    hasher.update_reader(fs::File::open(path).unwrap()).unwrap();
    hasher.finalize()
}
