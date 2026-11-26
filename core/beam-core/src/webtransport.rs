//! WebTransport: how a browser reaches the phone.
//!
//! Browsers can't open TCP or plain QUIC connections, but they can open a WebTransport session
//! (HTTP/3 over QUIC) to a server whose certificate they pin by hash. The phone runs this
//! server; the browser opens a session at [`PATH`] and then one bidirectional stream, which
//! carries Beam's Noise handshake and encrypted session exactly as TCP does. TLS only protects
//! the QUIC packets; who is on the other end is proven by the Noise handshake.

use std::collections::HashMap;
use std::io;
use std::net::{IpAddr, SocketAddr};
use std::pin::Pin;
use std::sync::{Arc, Mutex, PoisonError};
use std::task::{Context, Poll};
use std::time::Duration;

use bytes::Bytes;
use h3_webtransport::server::WebTransportSession;
use h3_webtransport::stream::BidiStream;
use quinn::crypto::rustls::QuicServerConfig;
use quinn::{Endpoint, Incoming, ServerConfig, VarInt};
use time::OffsetDateTime;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio::time::timeout;

use crate::webtransport_setup::open_session;
use crate::{BeamError, Result, quic, quic_tls, udp_socket, webtransport_cert};

/// Where a browser opens its WebTransport session.
pub const PATH: &str = "/beam";
/// HTTP/3's ALPN identifier, which browsers require.
const ALPN: &[u8] = b"h3";
/// A browser that connects but doesn't open its session and stream in time is dropped.
const SETUP_TIMEOUT: Duration = Duration::from_secs(10);
/// Browsers set up at once. Each has its own task, so a stalled one doesn't hold up the rest.
pub(crate) const MAX_SETTING_UP: usize = 8;
/// Setups at once from one address, so one device can't take every slot.
pub(crate) const MAX_SETTING_UP_PER_ADDRESS: usize = 2;
/// Browsers that finished setup and wait for [`WebTransportServer::accept`].
const READY_QUEUE: usize = 4;

pub(crate) type Session = WebTransportSession<h3_quinn::Connection, Bytes>;
type Ready = (BrowserStream, SocketAddr);

/// A server that accepts browsers' WebTransport sessions on one UDP socket.
pub struct WebTransportServer {
    endpoint: Endpoint,
    certificate_hash: [u8; 32],
    ready: tokio::sync::Mutex<mpsc::Receiver<Ready>>,
    accepting: JoinHandle<()>,
}

impl WebTransportServer {
    /// Binds to `addr` with a fresh certificate and starts setting up browsers that connect.
    /// Use port 0 to let the system pick a free port. Must be called inside a Tokio runtime.
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
        let tls = QuicServerConfig::try_from(tls).map_err(|err| setup(&err))?;
        let mut server = ServerConfig::with_crypto(Arc::new(tls));
        server.transport_config(Arc::new(quic::transport_config()));
        let endpoint = udp_socket::endpoint(server, addr).map_err(BeamError::network)?;
        let (ready_tx, ready_rx) = mpsc::channel(READY_QUEUE);
        let accepting = tokio::spawn(accept_loop(endpoint.clone(), ready_tx));
        Ok(Self {
            endpoint,
            certificate_hash: pinned.hash,
            ready: tokio::sync::Mutex::new(ready_rx),
            accepting,
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

    /// The next browser that opened its session and stream. Browsers whose setup fails or
    /// times out are dropped without reaching here: they haven't proven anything yet.
    ///
    /// # Errors
    ///
    /// A network error once the server is closed.
    pub async fn accept(&self) -> Result<(BrowserStream, SocketAddr)> {
        self.ready
            .lock()
            .await
            .recv()
            .await
            .ok_or_else(|| network("WebTransport server is closed"))
    }

    /// Waits until every browser has closed its connection, so what was sent last arrives.
    pub async fn wait_idle(&self) {
        self.endpoint.wait_idle().await;
    }

    /// Stops accepting and closes every session.
    pub fn close(&self) {
        self.accepting.abort();
        self.endpoint.close(VarInt::from_u32(0), b"closing");
    }
}

impl Drop for WebTransportServer {
    fn drop(&mut self) {
        self.close();
    }
}

/// Gives every incoming browser its own setup task, within the limits.
async fn accept_loop(endpoint: Endpoint, ready: mpsc::Sender<Ready>) {
    let slots = Arc::new(Mutex::new(Slots::default()));
    while let Some(incoming) = endpoint.accept().await {
        let from = incoming.remote_address();
        let Some(slot) = Slot::claim(&slots, from.ip()) else {
            incoming.refuse();
            continue;
        };
        let ready = ready.clone();
        tokio::spawn(async move {
            let _slot = slot;
            if let Ok(Ok(stream)) = timeout(SETUP_TIMEOUT, set_up(incoming)).await {
                // Fails only once the server is gone, and then the browser goes with it.
                let _ = ready.send((stream, from)).await;
            }
        });
    }
}

async fn set_up(incoming: Incoming) -> Result<BrowserStream> {
    let connection = incoming.await.map_err(network)?;
    open_session(connection).await
}

/// Setups in progress, in total and per address.
#[derive(Default)]
struct Slots {
    total: usize,
    by_address: HashMap<IpAddr, usize>,
}

/// One setup's place in [`Slots`], given back when the setup ends.
struct Slot {
    slots: Arc<Mutex<Slots>>,
    address: IpAddr,
}

impl Slot {
    fn claim(slots: &Arc<Mutex<Slots>>, address: IpAddr) -> Option<Self> {
        let mut now = slots.lock().unwrap_or_else(PoisonError::into_inner);
        let here = now.by_address.get(&address).copied().unwrap_or(0);
        if now.total >= MAX_SETTING_UP || here >= MAX_SETTING_UP_PER_ADDRESS {
            return None;
        }
        now.total += 1;
        now.by_address.insert(address, here + 1);
        Some(Self {
            slots: Arc::clone(slots),
            address,
        })
    }
}

impl Drop for Slot {
    fn drop(&mut self) {
        let mut now = self.slots.lock().unwrap_or_else(PoisonError::into_inner);
        now.total = now.total.saturating_sub(1);
        if let Some(here) = now.by_address.get_mut(&self.address) {
            *here = here.saturating_sub(1);
            if *here == 0 {
                now.by_address.remove(&self.address);
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

impl BrowserStream {
    pub(crate) const fn new(
        stream: BidiStream<h3_quinn::BidiStream<Bytes>, Bytes>,
        session: Arc<Session>,
    ) -> Self {
        Self {
            stream,
            _session: session,
        }
    }
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
pub(crate) fn network(err: impl ToString) -> BeamError {
    BeamError::network(io::Error::other(err.to_string()))
}

#[cfg(test)]
#[path = "webtransport_tests.rs"]
mod tests;
