//! QUIC endpoints on UDP sockets with large kernel buffers.
//!
//! A phone's default UDP receive buffer (about 320 KiB on Android) overflows while a fast
//! sender's packets arrive in bursts over Wi-Fi, and every overflow drops a packet that the
//! sender then counts as congestion. Asking for a few MiB each way removes those drops.

use std::io;
use std::net::{SocketAddr, UdpSocket};

use quinn::{Endpoint, EndpointConfig, ServerConfig};
use socket2::{Domain, Protocol, Socket, Type};

/// The buffer asked for, each way. Systems may grant less: Linux caps it at
/// `net.core.rmem_max`/`wmem_max`, and macOS refuses sizes above `kern.ipc.maxsockbuf`.
const BUFFER_BYTES: usize = 8 * 1024 * 1024;

/// Below this a request isn't worth making: the system's default is about as large.
const SMALLEST_REQUEST: usize = 256 * 1024;

/// A QUIC endpoint that accepts with `server`, on a socket bound by [`bind`]. Must be called
/// inside a Tokio runtime.
///
/// # Errors
///
/// The system's error if the socket can't be bound, or an error if there is no runtime.
pub(crate) fn endpoint(server: ServerConfig, addr: SocketAddr) -> io::Result<Endpoint> {
    let runtime =
        quinn::default_runtime().ok_or_else(|| io::Error::other("no Tokio runtime is running"))?;
    Endpoint::new(
        EndpointConfig::default(),
        Some(server),
        bind(addr)?,
        runtime,
    )
}

/// Binds a UDP socket to `addr` with the largest buffers the system allows, up to
/// [`BUFFER_BYTES`].
///
/// # Errors
///
/// The system's error if the socket can't be created or bound.
fn bind(addr: SocketAddr) -> io::Result<UdpSocket> {
    let socket = Socket::new(Domain::for_address(addr), Type::DGRAM, Some(Protocol::UDP))?;
    grow(|size| socket.set_recv_buffer_size(size));
    grow(|size| socket.set_send_buffer_size(size));
    socket.bind(&addr.into())?;
    Ok(socket.into())
}

/// Asks for [`BUFFER_BYTES`], halving while the system refuses. Large buffers only make
/// transfers faster, so if every size is refused the socket keeps the system's default.
fn grow(mut set: impl FnMut(usize) -> io::Result<()>) {
    let mut size = BUFFER_BYTES;
    while size >= SMALLEST_REQUEST && set(size).is_err() {
        size /= 2;
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use std::net::Ipv4Addr;

    use socket2::SockRef;

    use super::*;

    fn sizes(socket: &UdpSocket) -> (usize, usize) {
        let socket = SockRef::from(socket);
        (
            socket.recv_buffer_size().unwrap(),
            socket.send_buffer_size().unwrap(),
        )
    }

    #[test]
    fn buffers_are_larger_than_the_systems_default() {
        let addr = (Ipv4Addr::LOCALHOST, 0).into();
        let (default_recv, default_send) = sizes(&UdpSocket::bind(addr).unwrap());
        let (recv, send) = sizes(&bind(addr).unwrap());
        assert!(
            recv > default_recv,
            "receive buffer {recv} vs default {default_recv}"
        );
        assert!(
            send > default_send,
            "send buffer {send} vs default {default_send}"
        );
    }

    #[test]
    fn a_refused_size_is_halved_until_one_is_granted() {
        let mut asked = Vec::new();
        grow(|size| {
            asked.push(size);
            if size > BUFFER_BYTES / 4 {
                Err(io::ErrorKind::InvalidInput.into())
            } else {
                Ok(())
            }
        });
        assert_eq!(asked, [BUFFER_BYTES, BUFFER_BYTES / 2, BUFFER_BYTES / 4]);
    }

    #[test]
    fn every_size_refused_keeps_the_default() {
        let mut asked = 0;
        grow(|_| {
            asked += 1;
            Err(io::ErrorKind::InvalidInput.into())
        });
        assert_eq!(asked, 6, "8 MiB down to 256 KiB");
    }
}
