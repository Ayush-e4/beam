//! TCP transport: encrypted [`Connection`]s over TCP sockets.
//!
//! Nagle's algorithm is disabled because the session already batches data into large records,
//! and small control frames (acks, pings) should leave immediately.

use std::io;
use std::net::SocketAddr;
use std::time::Duration;

use tokio::net::{TcpListener, TcpStream};
use tokio::time::timeout;

use crate::connection::Connection;
use crate::handshake::Handshake;
use crate::{BeamError, Result};

/// How long an accepted connection has to complete the handshake.
pub const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);

/// Connects to `addr` and runs `handshake` as the initiator.
///
/// # Errors
///
/// Returns a network error if the peer is unreachable or connecting and the handshake take
/// longer than [`HANDSHAKE_TIMEOUT`], and a crypto error if it fails authentication.
pub async fn connect(addr: SocketAddr, handshake: Handshake) -> Result<Connection<TcpStream>> {
    within_handshake_timeout(async {
        let stream = TcpStream::connect(addr).await.map_err(BeamError::network)?;
        stream.set_nodelay(true).map_err(BeamError::network)?;
        Connection::establish(stream, handshake).await
    })
    .await
}

/// Fails with a network error if `work` takes longer than [`HANDSHAKE_TIMEOUT`]: a peer that
/// accepts the socket but never answers must not hang the caller.
async fn within_handshake_timeout<T>(work: impl Future<Output = Result<T>>) -> Result<T> {
    timeout(HANDSHAKE_TIMEOUT, work).await.map_err(|_| {
        BeamError::network(io::Error::new(
            io::ErrorKind::TimedOut,
            "handshake timed out",
        ))
    })?
}

/// Listens for incoming Beam connections over TCP.
pub struct TcpAcceptor {
    listener: TcpListener,
}

impl TcpAcceptor {
    /// Binds to `addr`. Use port 0 to let the system pick a free port.
    ///
    /// # Errors
    ///
    /// Returns a network error if the address cannot be bound.
    pub async fn bind(addr: SocketAddr) -> Result<Self> {
        let listener = TcpListener::bind(addr).await.map_err(BeamError::network)?;
        Ok(Self { listener })
    }

    /// The address actually bound, including the chosen port.
    ///
    /// # Errors
    ///
    /// Returns a network error if the socket cannot report its address.
    pub fn local_addr(&self) -> Result<SocketAddr> {
        self.listener.local_addr().map_err(BeamError::network)
    }

    /// Accepts the next connection and runs `handshake` as the responder, giving the peer
    /// [`HANDSHAKE_TIMEOUT`] to complete it so an idle connection can't block the listener.
    ///
    /// # Errors
    ///
    /// Returns a network error if accepting fails or the handshake times out, and a crypto
    /// error if the peer fails authentication.
    pub async fn accept(
        &self,
        handshake: Handshake,
    ) -> Result<(Connection<TcpStream>, SocketAddr)> {
        self.accept_from(handshake, |_| true).await
    }

    /// Like [`TcpAcceptor::accept`], but closes connections from addresses `allowed` refuses
    /// before any handshake work, so a blocked device costs next to nothing.
    ///
    /// # Errors
    ///
    /// As [`TcpAcceptor::accept`], plus a network error for a refused address.
    pub async fn accept_from(
        &self,
        handshake: Handshake,
        allowed: impl FnOnce(SocketAddr) -> bool,
    ) -> Result<(Connection<TcpStream>, SocketAddr)> {
        let (stream, from) = self.listener.accept().await.map_err(BeamError::network)?;
        if !allowed(from) {
            return Err(BeamError::network(io::Error::new(
                io::ErrorKind::ConnectionRefused,
                format!("connections from {} are refused for now", from.ip()),
            )));
        }
        stream.set_nodelay(true).map_err(BeamError::network)?;
        let established =
            within_handshake_timeout(Connection::establish(stream, handshake)).await?;
        Ok((established, from))
    }
}
