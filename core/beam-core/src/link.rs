//! A long-lived link between two paired devices that carries clipboard items both ways.
//!
//! [`run_link`] owns the connection while it runs: it sends what the app puts in `outgoing`,
//! hands what arrives to `incoming`, and keeps the link alive with `PING`/`PONG`. Either side
//! pings once it hasn't heard from the other for [`LinkTiming::ping_every`], and gives up when
//! the silence reaches [`LinkTiming::dead_after`], so a peer that vanished (Wi-Fi gone, tab
//! closed without a goodbye) ends the link instead of leaving it hanging.
//!
//! The same loop runs natively on tokio and in a browser worker (see [`crate::runtime`]).

use std::future::{Future, poll_fn};
use std::io;
use std::pin::pin;
use std::task::Poll;
use std::time::Duration;

use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::mpsc;

use crate::clipboard::{CLIPBOARD_STREAM, ClipItem};
use crate::frame::{Frame, FrameType};
use crate::runtime::{Instant, timeout};
use crate::{BeamError, Connection, Result};

/// When to ping and when to give up on a silent peer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LinkTiming {
    /// Ping after this long without hearing from the peer.
    pub ping_every: Duration,
    /// End the link after this long without hearing from the peer.
    pub dead_after: Duration,
}

impl Default for LinkTiming {
    /// Ping after 15 s of silence, give up after 45 s.
    fn default() -> Self {
        Self {
            ping_every: Duration::from_secs(15),
            dead_after: Duration::from_secs(45),
        }
    }
}

/// How a link ended without an error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkEnd {
    /// The peer closed the connection.
    PeerClosed,
    /// This side stopped: `outgoing` was closed, or `incoming` was dropped.
    Stopped,
}

enum Event {
    Outgoing(Option<ClipItem>),
    Arrived(Result<Option<Frame>>),
}

/// Runs the link until either side ends it. Closing `outgoing` (dropping its sender) ends it
/// cleanly: the connection is closed and the peer sees [`LinkEnd::PeerClosed`].
///
/// # Errors
///
/// A network error if the connection fails or the peer stays silent for
/// [`LinkTiming::dead_after`], and [`BeamError::Malformed`] if the peer sends a bad item or a
/// frame that doesn't belong on a link.
pub async fn run_link<S: AsyncRead + AsyncWrite + Unpin>(
    conn: &mut Connection<S>,
    mut outgoing: mpsc::Receiver<ClipItem>,
    incoming: mpsc::Sender<ClipItem>,
    timing: LinkTiming,
) -> Result<LinkEnd> {
    let mut last_heard = Instant::now();
    let mut pinged = false;
    loop {
        let silence = last_heard.elapsed();
        if silence >= timing.dead_after {
            return Err(silent_peer(timing.dead_after));
        }
        if silence >= timing.ping_every && !pinged {
            conn.send(&control(FrameType::Ping)?).await?;
            pinged = true;
        }
        let deadline = if pinged {
            timing.dead_after
        } else {
            timing.ping_every
        };
        let Some(event) = timeout(
            deadline.saturating_sub(silence),
            next_event(conn, &mut outgoing),
        )
        .await
        else {
            continue;
        };
        match event {
            Event::Outgoing(Some(item)) => conn.send(&item.to_frame()?).await?,
            Event::Outgoing(None) => {
                conn.close().await?;
                return Ok(LinkEnd::Stopped);
            }
            Event::Arrived(frame) => {
                let Some(frame) = frame? else {
                    return Ok(LinkEnd::PeerClosed);
                };
                last_heard = Instant::now();
                pinged = false;
                if !handle(conn, frame, &incoming).await? {
                    conn.close().await?;
                    return Ok(LinkEnd::Stopped);
                }
            }
        }
    }
}

/// Waits for whichever comes first: an item to send or a frame from the peer. Relies on
/// [`Connection::recv`] being cancel-safe.
async fn next_event<S: AsyncRead + AsyncWrite + Unpin>(
    conn: &mut Connection<S>,
    outgoing: &mut mpsc::Receiver<ClipItem>,
) -> Event {
    let mut arrived = pin!(conn.recv());
    poll_fn(|cx| {
        if let Poll::Ready(item) = outgoing.poll_recv(cx) {
            return Poll::Ready(Event::Outgoing(item));
        }
        arrived.as_mut().poll(cx).map(Event::Arrived)
    })
    .await
}

/// Acts on one frame from the peer. False if the app stopped listening.
async fn handle<S: AsyncRead + AsyncWrite + Unpin>(
    conn: &mut Connection<S>,
    frame: Frame,
    incoming: &mpsc::Sender<ClipItem>,
) -> Result<bool> {
    match frame.frame_type {
        FrameType::Clipboard => {
            let item = ClipItem::decode(&frame.payload)?;
            Ok(incoming.send(item).await.is_ok())
        }
        FrameType::Ping => {
            conn.send(&control(FrameType::Pong)?).await?;
            Ok(true)
        }
        FrameType::Pong => Ok(true),
        other => Err(BeamError::Malformed(format!(
            "unexpected {other:?} frame on a link"
        ))),
    }
}

fn control(frame_type: FrameType) -> Result<Frame> {
    Frame::new(frame_type, CLIPBOARD_STREAM, Vec::new())
}

fn silent_peer(after: Duration) -> BeamError {
    BeamError::network(io::Error::new(
        io::ErrorKind::TimedOut,
        format!(
            "the other device stopped answering for {}s",
            after.as_secs()
        ),
    ))
}

#[cfg(test)]
#[path = "link_tests.rs"]
mod tests;
