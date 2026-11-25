//! An encrypted connection over any reliable async byte stream.
//!
//! [`Connection::establish`] runs a Noise handshake over the stream, then [`Connection::send`]
//! and [`Connection::recv`] move frames through the encrypted [`Session`]. TCP sockets and QUIC
//! streams both plug in here, so the protocol logic is written once.
//!
//! Handshake messages travel as `length u16 BE | message`. Nothing here sets timeouts; callers
//! wrap calls in `tokio::time::timeout` to bound a stalled peer.

use std::collections::VecDeque;
use std::io;

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use crate::frame::Frame;
use crate::handshake::Handshake;
use crate::keys::PublicKey;
use crate::session::{LEN_PREFIX, Session};
use crate::verification::VerificationCode;
use crate::{BeamError, Result};

const READ_BUF_LEN: usize = 64 * 1024;

/// An authenticated, encrypted connection to one peer.
pub struct Connection<S> {
    stream: S,
    session: Session,
    code: VerificationCode,
    inbox: VecDeque<Frame>,
    read_buf: Box<[u8]>,
    out: Vec<u8>,
}

impl<S: AsyncRead + AsyncWrite + Unpin> Connection<S> {
    /// Runs `handshake` over `stream` and returns the encrypted connection.
    ///
    /// # Errors
    ///
    /// Returns a network error if the stream fails or closes, and a crypto error if the peer
    /// fails authentication.
    pub async fn establish(mut stream: S, mut handshake: Handshake) -> Result<Self> {
        while !handshake.is_finished() {
            if handshake.is_my_turn() {
                let message = handshake.write_message()?;
                write_handshake_message(&mut stream, &message).await?;
            } else {
                let message = read_handshake_message(&mut stream).await?;
                handshake.read_message(&message)?;
            }
        }
        let established = handshake.finish()?;
        let code = established.verification_code();
        Ok(Self {
            stream,
            session: Session::new(established),
            code,
            inbox: VecDeque::new(),
            read_buf: vec![0u8; READ_BUF_LEN].into_boxed_slice(),
            out: Vec::new(),
        })
    }

    /// The authenticated peer.
    #[must_use]
    pub const fn peer(&self) -> PublicKey {
        self.session.peer()
    }

    /// The code both users can compare to confirm a pairing.
    #[must_use]
    pub const fn verification_code(&self) -> VerificationCode {
        self.code
    }

    /// Encrypts and sends one frame.
    ///
    /// # Errors
    ///
    /// Returns a network error if the stream fails, or a session error if the session broke.
    pub async fn send(&mut self, frame: &Frame) -> Result<()> {
        self.out.clear();
        self.session.seal(frame, &mut self.out)?;
        self.stream
            .write_all(&self.out)
            .await
            .map_err(BeamError::network)?;
        self.stream.flush().await.map_err(BeamError::network)
    }

    /// Waits for the next frame. Returns `None` when the peer closed the connection cleanly
    /// between frames.
    ///
    /// Cancel-safe: dropping the future before it completes loses no data, so it can be used
    /// in `tokio::select!`, under a timeout, or polled once to check for a frame.
    ///
    /// # Errors
    ///
    /// Returns a network error if the stream fails or closes mid-frame, and a crypto or protocol
    /// error if the peer sends bad data.
    pub async fn recv(&mut self) -> Result<Option<Frame>> {
        loop {
            if let Some(frame) = self.inbox.pop_front() {
                return Ok(Some(frame));
            }
            let read = self
                .stream
                .read(&mut self.read_buf)
                .await
                .map_err(BeamError::network)?;
            if read == 0 {
                return if self.session.has_partial_input() {
                    Err(closed_mid_message())
                } else {
                    Ok(None)
                };
            }
            self.inbox
                .extend(self.session.receive(&self.read_buf[..read])?);
        }
    }

    /// Closes our sending side. The peer's [`Connection::recv`] then returns `None`.
    ///
    /// # Errors
    ///
    /// Returns a network error if the stream fails while closing.
    pub async fn close(&mut self) -> Result<()> {
        self.stream.shutdown().await.map_err(BeamError::network)
    }
}

async fn write_handshake_message<S: AsyncWrite + Unpin>(
    stream: &mut S,
    message: &[u8],
) -> Result<()> {
    let len = u16::try_from(message.len())
        .map_err(|_| BeamError::Handshake("handshake message too long".into()))?;
    let mut framed = Vec::with_capacity(LEN_PREFIX + message.len());
    framed.extend_from_slice(&len.to_be_bytes());
    framed.extend_from_slice(message);
    stream
        .write_all(&framed)
        .await
        .map_err(BeamError::network)?;
    stream.flush().await.map_err(BeamError::network)
}

async fn read_handshake_message<S: AsyncRead + Unpin>(stream: &mut S) -> Result<Vec<u8>> {
    let mut prefix = [0u8; LEN_PREFIX];
    stream
        .read_exact(&mut prefix)
        .await
        .map_err(BeamError::network)?;
    let mut message = vec![0u8; usize::from(u16::from_be_bytes(prefix))];
    stream
        .read_exact(&mut message)
        .await
        .map_err(BeamError::network)?;
    Ok(message)
}

fn closed_mid_message() -> BeamError {
    BeamError::network(io::Error::new(
        io::ErrorKind::UnexpectedEof,
        "connection closed in the middle of a message",
    ))
}

#[cfg(test)]
#[path = "connection_tests.rs"]
mod tests;
