//! Running a transfer over a [`Connection`], so the same code covers TCP and QUIC.
//!
//! - **Sender:** [`send_transfer`] offers the manifest, applies the receiver's `ACCEPT`,
//!   `RESUME` or `REJECT`, then streams chunks while reading `ACK`s as they arrive.
//! - **Receiver:** [`read_offer`] returns the manifest so the app can ask the user, then
//!   [`receive_transfer`] (or [`reject_offer`]) answers and receives (see [`receiver_side`]).
//! - **Done:** once every file is verified and published, the receiver sends an empty `ACK`.
//!   That tells the sender the files landed, not only that their chunks were acknowledged.
//! - **Already done:** if the connection drops after the receiver published but before the
//!   sender saw `done`, the sender reconnects and offers again. The receiving app, which knows
//!   that transfer finished, answers with [`confirm_received`] instead of receiving it twice.
//! - **Failures:** a side that fails for a reason other than the network sends `CANCEL`
//!   (failed) before giving up. After a dropped connection, reconnect and call again: the sender
//!   with the same [`Sender`], the receiver with a [`crate::Receiver`] reopened from its journal,
//!   which answers with `RESUME`.
//!
//! File I/O runs on tokio's blocking pool so it never stalls the async runtime. Nothing here
//! sets an overall timeout; callers wrap calls in `tokio::time::timeout` to bound a stalled peer.

use std::future::{Future, poll_fn};
use std::io;
use std::pin::pin;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::task::Poll;

use tokio::io::{AsyncRead, AsyncWrite};

use crate::frame::{Frame, FrameType};
use crate::manifest::TransferId;
use crate::messages::{Accept, Reason, Resume, decode_ack, decode_reason, encode_reason};
use crate::offer::encode_offer;
use crate::{BeamError, Connection, ErrorCategory, Result, Sender};

pub mod receiver_side;
pub use receiver_side::{
    confirm_received, read_offer, receive_transfer, receive_transfer_noting, reject_offer,
};

/// Stream ID used for every transfer frame (one transfer runs per connection).
pub const TRANSFER_STREAM: u32 = 1;

/// How a send ended when it did not fail.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SendOutcome {
    /// Every accepted file was verified and published by the receiver.
    Delivered,
    /// The receiver declined the offer.
    Rejected(Reason),
}

/// Offers `sender`'s manifest and sends it over `conn`. Keep the same `sender` to resume on a
/// new connection after a drop; other threads may lock it briefly to read progress.
///
/// # Errors
///
/// Returns a network error if the connection drops, [`BeamError::Cancelled`] if the receiver
/// cancels, [`BeamError::SourceChanged`] or storage errors for local files, and protocol
/// errors for bad replies.
pub async fn send_transfer<S: AsyncRead + AsyncWrite + Unpin>(
    conn: &mut Connection<S>,
    sender: &Arc<Mutex<Sender>>,
) -> Result<SendOutcome> {
    let (offer, transfer_id) = {
        let s = lock(sender);
        let manifest = s.manifest();
        (
            encode_offer(manifest, TRANSFER_STREAM)?,
            manifest.transfer_id(),
        )
    };
    let result = async {
        for frame in &offer {
            conn.send(frame).await?;
        }
        if let Some(reason) = await_reply(conn, sender, transfer_id).await? {
            return Ok(SendOutcome::Rejected(reason));
        }
        if stream_chunks(conn, sender, transfer_id).await? == Streamed::Acknowledged {
            await_done(conn, transfer_id).await?;
        }
        conn.close().await?;
        Ok(SendOutcome::Delivered)
    }
    .await;
    fail_with_cancel(conn, transfer_id, result).await
}

async fn await_reply<S: AsyncRead + AsyncWrite + Unpin>(
    conn: &mut Connection<S>,
    sender: &Arc<Mutex<Sender>>,
    transfer_id: TransferId,
) -> Result<Option<Reason>> {
    let frame = recv_frame(conn).await?;
    match frame.frame_type {
        FrameType::Accept => lock(sender).accept(&Accept::decode(&frame.payload)?)?,
        FrameType::Resume => lock(sender).resume(&Resume::decode(&frame.payload)?)?,
        FrameType::Reject => return reason_for(transfer_id, &frame.payload).map(Some),
        _ => return Err(unexpected(&frame, transfer_id)),
    }
    Ok(None)
}

async fn stream_chunks<S: AsyncRead + AsyncWrite + Unpin>(
    conn: &mut Connection<S>,
    sender: &Arc<Mutex<Sender>>,
    transfer_id: TransferId,
) -> Result<Streamed> {
    let mut exhausted = false;
    loop {
        if lock(sender).is_complete() {
            return Ok(Streamed::Acknowledged);
        }
        while let Some(frame) = try_recv(conn).await? {
            if is_done(sender, &frame, transfer_id)? {
                return confirm_done(sender).await;
            }
            if lock(sender).is_complete() {
                return Ok(Streamed::Acknowledged);
            }
        }
        let can_send = lock(sender).can_send();
        if can_send && !exhausted {
            let shared = Arc::clone(sender);
            match blocking(move || lock(&shared).next_chunk()).await? {
                Some(payload) => {
                    let frame = Frame::new(FrameType::Chunk, TRANSFER_STREAM, payload)?;
                    conn.send(&frame).await?;
                }
                None => exhausted = true,
            }
            continue;
        }
        let frame = recv_frame(conn).await?;
        if is_done(sender, &frame, transfer_id)? {
            return confirm_done(sender).await;
        }
    }
}

/// How streaming ended: every chunk acknowledged, or the receiver already said `done`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Streamed {
    Acknowledged,
    Done,
}

/// Applies an `ACK`. True if it is the receiver's `done` (an empty `ACK`).
fn is_done(sender: &Arc<Mutex<Sender>>, frame: &Frame, transfer_id: TransferId) -> Result<bool> {
    if frame.frame_type != FrameType::Ack {
        return Err(unexpected(frame, transfer_id));
    }
    let ranges = decode_ack(&frame.payload)?;
    if ranges.is_empty() {
        return Ok(true);
    }
    lock(sender).handle_ack(&ranges);
    Ok(false)
}

/// The receiver said `done`. It may say so right behind the last `ACK`, before the sender
/// looked for a next chunk (its window was full), so look now: `done` only holds if nothing is
/// left to send or to be acknowledged.
async fn confirm_done(sender: &Arc<Mutex<Sender>>) -> Result<Streamed> {
    let shared = Arc::clone(sender);
    let complete = blocking(move || {
        let mut sender = lock(&shared);
        Ok(sender.next_chunk()?.is_none() && sender.is_complete())
    })
    .await?;
    if complete {
        Ok(Streamed::Done)
    } else {
        Err(BeamError::Malformed(
            "receiver finished before every chunk was acknowledged".into(),
        ))
    }
}

async fn await_done<S: AsyncRead + AsyncWrite + Unpin>(
    conn: &mut Connection<S>,
    transfer_id: TransferId,
) -> Result<()> {
    let frame = recv_frame(conn).await?;
    if frame.frame_type == FrameType::Ack && decode_ack(&frame.payload)?.is_empty() {
        Ok(())
    } else {
        Err(unexpected(&frame, transfer_id))
    }
}

/// Locks the shared sender. A panic while it was locked cannot leave it unsafe to use: the next
/// `ACCEPT` or `RESUME` rebuilds its plan, so a poisoned lock is simply recovered.
fn lock(sender: &Mutex<Sender>) -> MutexGuard<'_, Sender> {
    sender.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Waits for the next frame; the peer closing the connection is an error mid-transfer.
pub(crate) async fn recv_frame<S: AsyncRead + AsyncWrite + Unpin>(
    conn: &mut Connection<S>,
) -> Result<Frame> {
    conn.recv().await?.ok_or_else(closed)
}

/// Returns a frame that has already arrived, without waiting. Relies on
/// [`Connection::recv`] being cancel-safe.
async fn try_recv<S: AsyncRead + AsyncWrite + Unpin>(
    conn: &mut Connection<S>,
) -> Result<Option<Frame>> {
    let mut recv = pin!(conn.recv());
    poll_fn(|cx| match recv.as_mut().poll(cx) {
        Poll::Ready(result) => {
            Poll::Ready(result.and_then(|frame| frame.ok_or_else(closed).map(Some)))
        }
        Poll::Pending => Poll::Ready(Ok(None)),
    })
    .await
}

/// Runs blocking file work on tokio's blocking pool.
pub(crate) async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T> + Send + 'static,
) -> Result<T> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|err| BeamError::storage(io::Error::other(format!("file task failed: {err}"))))?
}

/// A `REJECT` or `CANCEL` for this transfer; one for another transfer is a protocol error.
pub(crate) fn reason_for(transfer_id: TransferId, payload: &[u8]) -> Result<Reason> {
    let (id, reason) = decode_reason(payload)?;
    if id == transfer_id {
        Ok(reason)
    } else {
        Err(BeamError::Malformed("reply is for another transfer".into()))
    }
}

/// The error for a frame that should not arrive now. A `CANCEL` from the peer becomes
/// [`BeamError::Cancelled`].
pub(crate) fn unexpected(frame: &Frame, transfer_id: TransferId) -> BeamError {
    if frame.frame_type == FrameType::Cancel && reason_for(transfer_id, &frame.payload).is_ok() {
        return BeamError::Cancelled;
    }
    BeamError::Malformed(format!("unexpected {:?} frame", frame.frame_type))
}

/// Tells the peer we gave up, unless the link is gone or the peer cancelled first.
pub(crate) async fn fail_with_cancel<S: AsyncRead + AsyncWrite + Unpin, T>(
    conn: &mut Connection<S>,
    transfer_id: TransferId,
    result: Result<T>,
) -> Result<T> {
    let peer_should_know = result.as_ref().err().is_some_and(|err| {
        !matches!(
            err.category(),
            ErrorCategory::Network | ErrorCategory::Cancelled
        )
    });
    if peer_should_know {
        let payload = encode_reason(transfer_id, Reason::Failed);
        if let Ok(frame) = Frame::new(FrameType::Cancel, TRANSFER_STREAM, payload) {
            // Best effort: the original error is what the caller needs to see.
            let _ = conn.send(&frame).await;
        }
    }
    result
}

pub(crate) fn closed() -> BeamError {
    BeamError::network(io::Error::new(
        io::ErrorKind::ConnectionAborted,
        "the peer closed the connection mid-transfer",
    ))
}
