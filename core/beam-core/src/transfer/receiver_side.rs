//! The receiving side of [`crate::transfer`].
//!
//! [`receive_transfer`] answers with `ACCEPT`, or `RESUME` when the [`Receiver`] recovered
//! earlier progress, then verifies and writes chunks. It checkpoints and acknowledges every
//! [`ACK_EVERY_BYTES`], every [`ACK_INTERVAL`] while data is unacknowledged, and as soon as all
//! expected data has arrived, so the sender's window never stalls.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use tokio::io::{AsyncRead, AsyncWrite};

use super::{TRANSFER_STREAM, blocking, closed, fail_with_cancel, recv_frame, unexpected};
use crate::frame::{Frame, FrameType};
use crate::manifest::{Manifest, TransferId};
use crate::messages::{Accept, Reason, encode_ack, encode_reason, encode_resume};
use crate::offer::OfferAssembler;
use crate::{BeamError, Connection, Receiver, Result};

/// Acknowledge at least this often by volume (8 MiB, well under the sender's window).
pub const ACK_EVERY_BYTES: u64 = 8 * 1024 * 1024;
/// Acknowledge at least this often by time while data is unacknowledged.
pub const ACK_INTERVAL: Duration = Duration::from_millis(250);
/// How long to wait for the sender to close after the final "done" `ACK`.
const CLOSE_GRACE: Duration = Duration::from_secs(5);

/// Waits for the sender's `OFFER` and returns its manifest.
///
/// # Errors
///
/// Returns a network error if the connection drops, [`BeamError::Cancelled`] if the sender
/// cancels, and protocol errors for a bad offer.
pub async fn read_offer<S: AsyncRead + AsyncWrite + Unpin>(
    conn: &mut Connection<S>,
) -> Result<Manifest> {
    let mut assembler = OfferAssembler::new();
    loop {
        let frame = recv_frame(conn).await?;
        match frame.frame_type {
            FrameType::Offer => {
                if let Some(manifest) = assembler.push(&frame.payload)? {
                    return Ok(manifest);
                }
            }
            FrameType::Cancel => return Err(BeamError::Cancelled),
            other => {
                return Err(BeamError::Malformed(format!(
                    "expected an offer, got {other:?}"
                )));
            }
        }
    }
}

/// Declines an offer and closes the connection.
///
/// # Errors
///
/// Returns a network error if sending fails.
pub async fn reject_offer<S: AsyncRead + AsyncWrite + Unpin>(
    conn: &mut Connection<S>,
    transfer_id: TransferId,
    reason: Reason,
) -> Result<()> {
    let payload = encode_reason(transfer_id, reason);
    conn.send(&Frame::new(FrameType::Reject, TRANSFER_STREAM, payload)?)
        .await?;
    conn.close().await
}

/// Accepts (or resumes) the offer `receiver` was opened for and receives it. Returns the final
/// paths of the published files, in manifest order.
///
/// # Errors
///
/// Returns a network error if the connection drops (reopen the [`Receiver`] from its journal
/// and call again on a new connection), [`BeamError::Cancelled`] if the sender cancels,
/// integrity errors for bad data, and storage errors.
pub async fn receive_transfer<S: AsyncRead + AsyncWrite + Unpin>(
    conn: &mut Connection<S>,
    receiver: Receiver,
) -> Result<Vec<PathBuf>> {
    let transfer_id = receiver.manifest().transfer_id();
    let result = async {
        conn.send(&reply_for(&receiver)?).await?;
        receive_chunks(conn, receiver).await
    }
    .await;
    let paths = fail_with_cancel(conn, transfer_id, result).await?;

    // The files are published; from here on a failure no longer matters to the outcome.
    let done = Frame::new(FrameType::Ack, TRANSFER_STREAM, encode_ack(&[])?)?;
    if conn.send(&done).await.is_ok() {
        let drain = async { while let Ok(Some(_)) = conn.recv().await {} };
        let _ = tokio::time::timeout(CLOSE_GRACE, drain).await;
    }
    let _ = conn.close().await;
    Ok(paths)
}

fn reply_for(receiver: &Receiver) -> Result<Frame> {
    let transfer_id = receiver.manifest().transfer_id();
    if receiver.resumed() {
        let payload = encode_resume(transfer_id, &receiver.have())?;
        return Frame::new(FrameType::Resume, TRANSFER_STREAM, payload);
    }
    let accept = Accept {
        transfer_id,
        items: receiver.accepted_items(),
    };
    Frame::new(FrameType::Accept, TRANSFER_STREAM, accept.encode()?)
}

async fn receive_chunks<S: AsyncRead + AsyncWrite + Unpin>(
    conn: &mut Connection<S>,
    mut receiver: Receiver,
) -> Result<Vec<PathBuf>> {
    let transfer_id = receiver.manifest().transfer_id();
    let mut unacked: u64 = 0;
    let mut last_ack = Instant::now();
    loop {
        let due = unacked >= ACK_EVERY_BYTES
            || (unacked > 0 && last_ack.elapsed() >= ACK_INTERVAL)
            || receiver.has_all_data();
        if due {
            receiver = checkpoint(conn, receiver).await?;
            (unacked, last_ack) = (0, Instant::now());
            if receiver.is_complete() {
                return blocking(move || receiver.finish()).await;
            }
        }

        let next = if unacked == 0 {
            Some(conn.recv().await?)
        } else {
            let wait = ACK_INTERVAL.saturating_sub(last_ack.elapsed());
            tokio::time::timeout(wait, conn.recv())
                .await
                .ok()
                .transpose()?
        };
        let Some(frame) = next else {
            continue; // the ack interval passed with nothing new
        };
        let frame = frame.ok_or_else(closed)?;
        if frame.frame_type != FrameType::Chunk {
            return Err(unexpected(&frame, transfer_id));
        }
        unacked += u64::try_from(frame.payload.len()).unwrap_or(u64::MAX);
        receiver = blocking(move || {
            receiver.handle_chunk(&frame.payload)?;
            Ok(receiver)
        })
        .await?;
    }
}

/// Makes received data durable and acknowledges it.
async fn checkpoint<S: AsyncRead + AsyncWrite + Unpin>(
    conn: &mut Connection<S>,
    mut receiver: Receiver,
) -> Result<Receiver> {
    let (receiver, acked) = blocking(move || {
        let acked = receiver.checkpoint()?;
        Ok((receiver, acked))
    })
    .await?;
    if !acked.is_empty() {
        let frame = Frame::new(FrameType::Ack, TRANSFER_STREAM, encode_ack(&acked)?)?;
        conn.send(&frame).await?;
    }
    Ok(receiver)
}
