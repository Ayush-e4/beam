//! TCP transport: encrypted [`Connection`]s over TCP sockets.
//!
//! Nagle's algorithm is disabled because the session already batches data into large records,
//! and small control frames (acks, pings) should leave immediately.

use std::io;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use tokio::net::{TcpListener, TcpSocket, TcpStream};
use tokio::time::timeout;

use crate::connection::Connection;
use crate::handshake::Handshake;
use crate::{BeamError, Result};

/// How long an accepted connection has to complete the handshake.
pub const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);

/// Something done to a socket before it connects, such as sending it over one particular
/// network instead of the system's default (which may be a VPN).
pub type PrepareSocket = Arc<dyn Fn(&TcpSocket) -> io::Result<()> + Send + Sync>;

/// Connects to `addr` and runs `handshake` as the initiator.
///
/// # Errors
///
/// Returns a network error if the peer is unreachable or connecting and the handshake take
/// longer than [`HANDSHAKE_TIMEOUT`], and a crypto error if it fails authentication.
pub async fn connect(addr: SocketAddr, handshake: Handshake) -> Result<Connection<TcpStream>> {
    connect_prepared(addr, handshake, None).await
}

/// As [`connect`], running `prepare` on the socket first.
///
/// # Errors
///
/// As [`connect`], and a network error if `prepare` fails.
pub async fn connect_prepared(
    addr: SocketAddr,
    handshake: Handshake,
    prepare: Option<PrepareSocket>,
) -> Result<Connection<TcpStream>> {
    within_handshake_timeout(async {
        let socket = if addr.is_ipv4() {
            TcpSocket::new_v4()
        } else {
            TcpSocket::new_v6()
        }
        .map_err(BeamError::network)?;
        if let Some(prepare) = prepare {
            prepare(&socket).map_err(BeamError::network)?;
        }
        let stream = socket.connect(addr).await.map_err(BeamError::network)?;
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
        let (stream, from) = self.accept_socket().await?;
        Ok((respond(stream, handshake).await?, from))
    }

    /// Accepts the next connection without any handshake work, so the caller can check the
    /// address first and run [`respond`] for many devices at once.
    ///
    /// # Errors
    ///
    /// Returns a network error if accepting fails.
    pub async fn accept_socket(&self) -> Result<(TcpStream, SocketAddr)> {
        self.listener.accept().await.map_err(BeamError::network)
    }
}

/// Runs `handshake` as the responder on an accepted `stream`, giving the peer
/// [`HANDSHAKE_TIMEOUT`] to complete it.
///
/// # Errors
///
/// Returns a network error if the handshake times out or the link fails, and a crypto error
/// if the peer fails authentication.
pub async fn respond(stream: TcpStream, handshake: Handshake) -> Result<Connection<TcpStream>> {
    stream.set_nodelay(true).map_err(BeamError::network)?;
    within_handshake_timeout(Connection::establish(stream, handshake)).await
}
