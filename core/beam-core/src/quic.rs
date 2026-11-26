//! QUIC transport: encrypted [`Connection`]s over QUIC (`quinn`).
//!
//! One [`QuicEndpoint`] both accepts and opens connections on a single UDP socket. Each Beam
//! connection uses one bidirectional QUIC stream carrying the Noise handshake and the encrypted
//! session, exactly like TCP. Congestion control is BBR with large flow-control windows, per
//! `docs/performance.md`.
//!
//! [`Connection::close`] finishes our side of the stream; keep the connection alive until the
//! peer has read everything (for example until its own `recv` returns `None`), since dropping
//! every handle closes the QUIC connection immediately.

use std::io;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use quinn::crypto::rustls::{QuicClientConfig, QuicServerConfig};
use quinn::{ClientConfig, Endpoint, IdleTimeout, RecvStream, SendStream, ServerConfig, VarInt};
use tokio::io::Join;

use crate::connection::Connection;
use crate::handshake::Handshake;
use crate::quic_tls::{self, SERVER_NAME};
use crate::{BeamError, Result};

/// The byte stream a QUIC [`Connection`] runs over.
pub type QuicStream = Join<RecvStream, SendStream>;

const STREAM_WINDOW: u32 = 16 * 1024 * 1024;
const CONNECTION_WINDOW: u32 = 32 * 1024 * 1024;
const KEEP_ALIVE: Duration = Duration::from_secs(5);
const IDLE_TIMEOUT: Duration = Duration::from_secs(30);

/// A UDP socket that accepts and opens Beam connections over QUIC.
pub struct QuicEndpoint {
    endpoint: Endpoint,
}

impl QuicEndpoint {
    /// Binds to `addr`. Use port 0 to let the system pick a free port. Must be called inside a
    /// Tokio runtime.
    ///
    /// # Errors
    ///
    /// Returns a network error if the socket cannot be bound and a crypto error if TLS setup
    /// fails.
    pub fn bind(addr: SocketAddr) -> Result<Self> {
        let transport = Arc::new(transport_config());
        let server_tls = QuicServerConfig::try_from(quic_tls::server_config()?)
            .map_err(|err| BeamError::Handshake(format!("QUIC server setup failed: {err}")))?;
        let mut server = ServerConfig::with_crypto(Arc::new(server_tls));
        server.transport_config(transport.clone());
        let client_tls = QuicClientConfig::try_from(quic_tls::client_config()?)
            .map_err(|err| BeamError::Handshake(format!("QUIC client setup failed: {err}")))?;
        let mut client = ClientConfig::new(Arc::new(client_tls));
        client.transport_config(transport);

        let mut endpoint = Endpoint::server(server, addr).map_err(BeamError::network)?;
        endpoint.set_default_client_config(client);
        Ok(Self { endpoint })
    }

    /// The address actually bound, including the chosen port.
    ///
    /// # Errors
    ///
    /// Returns a network error if the socket cannot report its address.
    pub fn local_addr(&self) -> Result<SocketAddr> {
        self.endpoint.local_addr().map_err(BeamError::network)
    }

    /// Connects to `addr` and runs `handshake` as the initiator.
    ///
    /// # Errors
    ///
    /// Returns a network error if the peer is unreachable, and a crypto error if it fails
    /// authentication.
    pub async fn connect(
        &self,
        addr: SocketAddr,
        handshake: Handshake,
    ) -> Result<Connection<QuicStream>> {
        let connection = self
            .endpoint
            .connect(addr, SERVER_NAME)
            .map_err(network)?
            .await
            .map_err(network)?;
        let (send, recv) = connection.open_bi().await.map_err(network)?;
        Connection::establish(tokio::io::join(recv, send), handshake).await
    }

    /// Accepts the next connection and runs `handshake` as the responder.
    ///
    /// # Errors
    ///
    /// Returns a network error if the endpoint is closed or the connection fails, and a crypto
    /// error if the peer fails authentication.
    pub async fn accept(
        &self,
        handshake: Handshake,
    ) -> Result<(Connection<QuicStream>, SocketAddr)> {
        let incoming = self
            .endpoint
            .accept()
            .await
            .ok_or_else(|| network("QUIC endpoint is closed"))?;
        let from = incoming.remote_address();
        let connection = incoming.await.map_err(network)?;
        let (send, recv) = connection.accept_bi().await.map_err(network)?;
        let secured = Connection::establish(tokio::io::join(recv, send), handshake).await?;
        Ok((secured, from))
    }

    /// Stops accepting and closes every connection on this endpoint.
    pub fn close(&self) {
        self.endpoint.close(VarInt::from_u32(0), b"closing");
    }
}

pub(crate) fn transport_config() -> quinn::TransportConfig {
    let mut config = quinn::TransportConfig::default();
    config
        .congestion_controller_factory(Arc::new(quinn::congestion::BbrConfig::default()))
        .stream_receive_window(VarInt::from_u32(STREAM_WINDOW))
        .receive_window(VarInt::from_u32(CONNECTION_WINDOW))
        .send_window(u64::from(CONNECTION_WINDOW))
        .keep_alive_interval(Some(KEEP_ALIVE))
        .max_idle_timeout(IdleTimeout::try_from(IDLE_TIMEOUT).ok());
    config
}

#[allow(clippy::needless_pass_by_value)] // used as `map_err(network)`
fn network(err: impl ToString) -> BeamError {
    BeamError::network(io::Error::other(err.to_string()))
}
