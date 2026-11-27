//! The byte stream under a signaling room's WebSocket: plain TCP for a local service, TLS for
//! the deployed one.

use std::io;
use std::pin::Pin;
use std::task::{Context, Poll};

use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::TcpStream;
use tokio_rustls::client::TlsStream;

/// TCP, with or without TLS.
pub(crate) enum RoomStream {
    Plain(TcpStream),
    Tls(Box<TlsStream<TcpStream>>),
}

impl AsyncRead for RoomStream {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        match self.get_mut() {
            Self::Plain(tcp) => Pin::new(tcp).poll_read(cx, buf),
            Self::Tls(tls) => Pin::new(tls.as_mut()).poll_read(cx, buf),
        }
    }
}

impl AsyncWrite for RoomStream {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        match self.get_mut() {
            Self::Plain(tcp) => Pin::new(tcp).poll_write(cx, buf),
            Self::Tls(tls) => Pin::new(tls.as_mut()).poll_write(cx, buf),
        }
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match self.get_mut() {
            Self::Plain(tcp) => Pin::new(tcp).poll_flush(cx),
            Self::Tls(tls) => Pin::new(tls.as_mut()).poll_flush(cx),
        }
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match self.get_mut() {
            Self::Plain(tcp) => Pin::new(tcp).poll_shutdown(cx),
            Self::Tls(tls) => Pin::new(tls.as_mut()).poll_shutdown(cx),
        }
    }
}
