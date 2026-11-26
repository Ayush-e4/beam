//! A receiver that finishes the moment the last chunk lands (a browser staging in OPFS, or any
//! fast disk) sends its last `ACK` and its `done` back to back. A sender with a small window
//! has just sent the last chunk then, and hasn't yet looked for more to send: it must still
//! take `done` as delivered, not as the receiver giving up early.

#![allow(clippy::unwrap_used)]

use std::io;
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use beam_core::messages::{Accept, encode_ack};
use beam_core::transfer::TRANSFER_STREAM;
use beam_core::{
    Cipher, Connection, Frame, FrameType, Handshake, ReadAt, ReadableFile, Receiver, SendOutcome,
    Sender, StaticKeypair, prepare_readable, read_offer, send_transfer,
};
use tempfile::TempDir;
use tokio::io::{DuplexStream, duplex};

#[derive(Debug)]
struct Memory(Vec<u8>);

impl ReadAt for Memory {
    fn size(&self) -> io::Result<u64> {
        Ok(self.0.len() as u64)
    }

    fn modified(&self) -> io::Result<Option<SystemTime>> {
        Ok(None)
    }

    fn read_exact_at(&self, buf: &mut [u8], offset: u64) -> io::Result<()> {
        let start = usize::try_from(offset).unwrap();
        buf.copy_from_slice(&self.0[start..start + buf.len()]);
        Ok(())
    }
}

/// Three chunks (4 MiB each) of bytes that don't compress.
fn noise() -> Vec<u8> {
    let mut state = 0x2545_F491_4F6C_DD1D_u64;
    (0..9_000_000)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state.to_le_bytes()[0]
        })
        .collect()
}

async fn ack(conn: &mut Connection<DuplexStream>, ranges: &[beam_core::messages::ChunkRange]) {
    let frame = Frame::new(FrameType::Ack, TRANSFER_STREAM, encode_ack(ranges).unwrap());
    conn.send(&frame.unwrap()).await.unwrap();
}

/// Acknowledges every chunk at once, and on the last one finishes and says `done` straight
/// after the final `ACK`, without giving the sender a moment in between. If `quit_early`, it
/// says `done` after the first chunk instead, which the sender must not believe.
async fn fast_receiver(mut conn: Connection<DuplexStream>, dest: &TempDir, quit_early: bool) {
    let manifest = read_offer(&mut conn).await.unwrap();
    let accept = Accept {
        transfer_id: manifest.transfer_id(),
        items: Vec::new(),
    };
    let frame = Frame::new(FrameType::Accept, TRANSFER_STREAM, accept.encode().unwrap());
    conn.send(&frame.unwrap()).await.unwrap();
    let mut receiver = Receiver::open(manifest, dest.path(), &[]).unwrap();
    while !receiver.is_complete() {
        let chunk = conn.recv().await.unwrap().unwrap();
        receiver.handle_chunk(&chunk.payload).unwrap();
        let acked = receiver.checkpoint().unwrap();
        ack(&mut conn, &acked).await;
        if quit_early {
            break;
        }
    }
    if !quit_early {
        receiver.finish().unwrap();
    }
    ack(&mut conn, &[]).await; // done
    while let Ok(Some(_)) = conn.recv().await {}
}

/// Sends one file with one chunk in flight at a time (the last chunk fills the window) to a
/// [`fast_receiver`].
async fn send_to_fast_receiver(quit_early: bool) -> beam_core::Result<SendOutcome> {
    let file = ReadableFile {
        name: "video.mp4".into(),
        reader: Arc::new(Memory(noise())),
    };
    let prepared = prepare_readable(vec![file]).unwrap();
    let dest = TempDir::new().unwrap();
    let (phone_end, laptop_end) = duplex(4 * 1024 * 1024);
    let (phone, laptop) = (
        StaticKeypair::generate().unwrap(),
        StaticKeypair::generate().unwrap(),
    );
    let laptop_key = laptop.public();

    let sending = async {
        let handshake = Handshake::known_initiator(&phone, Cipher::ChaChaPoly, laptop_key);
        let mut conn = Connection::establish(phone_end, handshake.unwrap())
            .await
            .unwrap();
        let sender = Arc::new(Mutex::new(Sender::with_window(prepared, 1)));
        send_transfer(&mut conn, &sender).await
    };
    let receiving = async {
        let handshake = Handshake::known_responder(&laptop, Cipher::ChaChaPoly).unwrap();
        let conn = Connection::establish(laptop_end, handshake).await.unwrap();
        fast_receiver(conn, &dest, quit_early).await;
    };
    let (sent, ()) = tokio::join!(sending, receiving);
    if !quit_early {
        assert_eq!(
            std::fs::read(dest.path().join("video.mp4")).unwrap(),
            noise()
        );
    }
    sent
}

#[tokio::test]
async fn a_done_right_behind_the_last_ack_still_counts_as_delivered() {
    assert_eq!(
        send_to_fast_receiver(false).await.unwrap(),
        SendOutcome::Delivered
    );
}

#[tokio::test]
async fn a_done_before_every_chunk_was_sent_is_refused() {
    let err = send_to_fast_receiver(true).await.unwrap_err();
    assert!(err.to_string().contains("before every chunk"), "{err}");
}
