//! WebTransport: how a browser reaches the phone.
//!
//! Browsers can't open TCP or plain QUIC connections, but they can open a WebTransport session
//! (HTTP/3 over QUIC) to a server whose certificate they pin by hash. The phone runs this
//! server; the browser opens a session at [`PATH`] and then one bidirectional stream, which
//! carries Beam's Noise handshake and encrypted session exactly as TCP does. TLS only protects
//! the QUIC packets; who is on the other end is proven by the Noise handshake.

use std::io;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;

use bytes::Bytes;
use h3::ext::Protocol;
use h3_webtransport::server::{AcceptedBi, WebTransportSession};
use h3_webtransport::stream::BidiStream;
use http::{Method, Response, StatusCode};
use quinn::crypto::rustls::QuicServerConfig;
use quinn::{Endpoint, ServerConfig, VarInt};
use time::OffsetDateTime;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::time::timeout;

use crate::{BeamError, Result, quic, quic_tls, webtransport_cert};

/// Where a browser opens its WebTransport session.
pub const PATH: &str = "/beam";
/// HTTP/3's ALPN identifier, which browsers require.
const ALPN: &[u8] = b"h3";
/// A browser that connects but doesn't open its session and stream in time is dropped.
const SETUP_TIMEOUT: Duration = Duration::from_secs(10);

type Session = WebTransportSession<h3_quinn::Connection, Bytes>;

/// A server that accepts browsers' WebTransport sessions on one UDP socket.
pub struct WebTransportServer {
    endpoint: Endpoint,
    certificate_hash: [u8; 32],
}

impl WebTransportServer {
    /// Binds to `addr` with a fresh certificate. Use port 0 to let the system pick a free port.
    /// Must be called inside a Tokio runtime.
    ///
    /// # Errors
    ///
    /// A network error if the socket can't be bound, a crypto error if TLS setup fails.
    pub fn bind(addr: SocketAddr) -> Result<Self> {
        let pinned = webtransport_cert::generate(OffsetDateTime::now_utc())?;
        let mut tls = rustls::ServerConfig::builder_with_provider(quic_tls::provider())
            .with_protocol_versions(&[&rustls::version::TLS13])
            .map_err(|err| setup(&err))?
            .with_no_client_auth()
            .with_single_cert(vec![pinned.certificate], pinned.key)
            .map_err(|err| setup(&err))?;
        tls.alpn_protocols = vec![ALPN.to_vec()];
        tls.max_early_data_size = u32::MAX;
        let tls = QuicServerConfig::try_from(tls).map_err(|err| setup(&err))?;
        let mut server = ServerConfig::with_crypto(Arc::new(tls));
        server.transport_config(Arc::new(quic::transport_config()));
        let endpoint = Endpoint::server(server, addr).map_err(BeamError::network)?;
        Ok(Self {
            endpoint,
            certificate_hash: pinned.hash,
        })
    }

    /// The SHA-256 of this server's certificate, which browsers pin.
    #[must_use]
    pub const fn certificate_hash(&self) -> [u8; 32] {
        self.certificate_hash
    }

    /// The address actually bound, including the chosen port.
    ///
    /// # Errors
    ///
    /// A network error if the socket can't report its address.
    pub fn local_addr(&self) -> Result<SocketAddr> {
        self.endpoint.local_addr().map_err(BeamError::network)
    }

    /// Waits for the next browser and returns the stream its session opened. Only that one
    /// browser's setup is lost if it fails; call again for the next.
    ///
    /// # Errors
    ///
    /// A network error if the server is closed or the browser's setup fails or times out.
    pub async fn accept(&self) -> Result<(BrowserStream, SocketAddr)> {
        let incoming = self
            .endpoint
            .accept()
            .await
            .ok_or_else(|| network("WebTransport server is closed"))?;
        let from = incoming.remote_address();
        let stream = timeout(SETUP_TIMEOUT, async {
            let connection = incoming.await.map_err(network)?;
            open_session(connection).await
        })
        .await
        .map_err(|_| network("browser didn't open its WebTransport session in time"))??;
        Ok((stream, from))
    }

    /// Waits until every browser has closed its connection, so what was sent last arrives.
    pub async fn wait_idle(&self) {
        self.endpoint.wait_idle().await;
    }

    /// Stops accepting and closes every session.
    pub fn close(&self) {
        self.endpoint.close(VarInt::from_u32(0), b"closing");
    }
}

/// Runs HTTP/3 on `connection` until the browser asks for a WebTransport session at
/// [`PATH`], then waits for the session's first bidirectional stream.
async fn open_session(connection: quinn::Connection) -> Result<BrowserStream> {
    let mut h3 = h3::server::builder()
        .enable_webtransport(true)
        .enable_extended_connect(true)
        .enable_datagram(true)
        .max_webtransport_sessions(1)
        .send_grease(true)
        .build(h3_quinn::Connection::new(connection))
        .await
        .map_err(network)?;
    loop {
        let resolver = h3
            .accept()
            .await
            .map_err(network)?
            .ok_or_else(|| network("browser closed before opening a session"))?;
        let (request, mut stream) = resolver.resolve_request().await.map_err(network)?;
        let wants_session = request.method() == Method::CONNECT
            && request.extensions().get::<Protocol>() == Some(&Protocol::WEB_TRANSPORT)
            && request.uri().path() == PATH;
        if !wants_session {
            let refused = Response::builder()
                .status(StatusCode::NOT_FOUND)
                .body(())
                .map_err(network)?;
            stream.send_response(refused).await.map_err(network)?;
            stream.finish().await.map_err(network)?;
            continue;
        }
        let session = Arc::new(
            WebTransportSession::accept(request, stream, h3)
                .await
                .map_err(network)?,
        );
        return first_stream(session).await;
    }
}

async fn first_stream(session: Arc<Session>) -> Result<BrowserStream> {
    loop {
        match session.accept_bi().await.map_err(network)? {
            Some(AcceptedBi::BidiStream(_, stream)) => {
                return Ok(BrowserStream {
                    stream,
                    _session: session,
                });
            }
            // A plain HTTP request inside the session: not something Beam serves.
            Some(AcceptedBi::Request(_, mut stream)) => {
                let refused = Response::builder()
                    .status(StatusCode::NOT_FOUND)
                    .body(())
                    .map_err(network)?;
                stream.send_response(refused).await.map_err(network)?;
            }
            None => {
                return Err(network(
                    "browser closed its session before opening a stream",
                ));
            }
        }
    }
}

/// The bidirectional stream a browser's session opened. It keeps the session open while it
/// lives; hand it to [`crate::Connection::establish`] like a TCP stream.
pub struct BrowserStream {
    stream: BidiStream<h3_quinn::BidiStream<Bytes>, Bytes>,
    _session: Arc<Session>,
}

impl AsyncRead for BrowserStream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.stream).poll_read(cx, buf)
    }
}

impl AsyncWrite for BrowserStream {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.stream).poll_write(cx, buf)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.stream).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.stream).poll_shutdown(cx)
    }
}

fn setup(err: &dyn std::fmt::Display) -> BeamError {
    BeamError::Handshake(format!("WebTransport setup failed: {err}"))
}

#[allow(clippy::needless_pass_by_value)] // used as `map_err(network)`
fn network(err: impl ToString) -> BeamError {
    BeamError::network(io::Error::other(err.to_string()))
}
