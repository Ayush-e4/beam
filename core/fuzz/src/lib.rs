//! Shared setup for the fuzz targets. Fuzzing setup code may panic: a broken fixture should
//! stop the run loudly rather than hide behind an error path.

use beam_core::manifest::{ManifestItem, TransferId};
use beam_core::{Cipher, Handshake, Manifest, SafePath, Session, StaticKeypair};

const TOKEN: &[u8; 16] = b"fuzzing-token-01";

/// A manifest with a mix of small (packable) and larger files.
#[must_use]
pub fn manifest() -> Manifest {
    let item = |item_id: u32, size: u64| ManifestItem {
        item_id,
        path: SafePath::parse(&format!("dir/file-{item_id}.bin")).expect("valid path"),
        size,
        mime: "application/octet-stream".into(),
        modified_at_ms: None,
        chunk_size: 64 * 1024,
    };
    let items = vec![
        item(1, 0),
        item(2, 10),
        item(3, 300),
        item(4, 5000),
        item(5, 200_000),
    ];
    let total = items.iter().map(|i| i.size).sum();
    Manifest::new(TransferId([7; 16]), items, total).expect("valid manifest")
}

/// Two sessions that completed a real pairing handshake: (initiator, responder).
#[must_use]
pub fn paired_sessions() -> (Session, Session) {
    let scanner = StaticKeypair::generate().expect("key");
    let phone = StaticKeypair::generate().expect("key");
    let mut i = Handshake::pairing_initiator(&scanner, Cipher::AesGcm, TOKEN, phone.public())
        .expect("initiator");
    let mut r = Handshake::pairing_responder(&phone, Cipher::AesGcm, TOKEN).expect("responder");
    r.read_message(&i.write_message().expect("m1")).expect("m1");
    i.read_message(&r.write_message().expect("m2")).expect("m2");
    r.read_message(&i.write_message().expect("m3")).expect("m3");
    (
        Session::new(i.finish().expect("done")),
        Session::new(r.finish().expect("done")),
    )
}

/// A fresh pairing responder waiting for the first handshake message.
#[must_use]
pub fn pairing_responder() -> Handshake {
    let phone = StaticKeypair::generate().expect("key");
    Handshake::pairing_responder(&phone, Cipher::ChaChaPoly, TOKEN).expect("responder")
}

/// Splits `data` into pieces whose sizes come from the input itself, so the fuzzer also
/// explores how parsers handle data arriving in fragments.
pub fn split(data: &[u8]) -> impl Iterator<Item = &[u8]> {
    let mut rest = data;
    std::iter::from_fn(move || {
        let (&size, tail) = rest.split_first()?;
        let take = (usize::from(size) * 37 + 1).min(tail.len());
        let (piece, tail) = tail.split_at(take);
        rest = tail;
        Some(piece)
    })
}
