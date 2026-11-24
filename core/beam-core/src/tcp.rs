//! TCP transport: encrypted [`Connection`]s over TCP sockets.
//!
//! Nagle's algorithm is disabled because the session already batches data into large records,
//! and small control frames (acks, pings) should leave immediately.

use std::net::SocketAddr;

use tokio::net::{TcpListener, TcpStream};

use crate::connection::Connection;
use crate::handshake::Handshake;
use crate::{BeamError, Result};

/// Connects to `addr` and runs `handshake` as the initiator.
///
/// # Errors
///
/// Returns a network error if the peer is unreachable, and a crypto error if it fails
/// authentication.
pub async fn connect(addr: SocketAddr, handshake: Handshake) -> Result<Connection<TcpStream>> {
    let stream = TcpStream::connect(addr).await.map_err(BeamError::network)?;
    stream.set_nodelay(true).map_err(BeamError::network)?;
    Connection::establish(stream, handshake).await
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

    /// Accepts the next connection and runs `handshake` as the responder.
    ///
    /// # Errors
    ///
    /// Returns a network error if accepting fails, and a crypto error if the peer fails
    /// authentication.
    pub async fn accept(
        &self,
        handshake: Handshake,
    ) -> Result<(Connection<TcpStream>, SocketAddr)> {
        let (stream, from) = self.listener.accept().await.map_err(BeamError::network)?;
        stream.set_nodelay(true).map_err(BeamError::network)?;
        Ok((Connection::establish(stream, handshake).await?, from))
    }
}
