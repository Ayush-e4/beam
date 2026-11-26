//! Sending connections to nearby devices over one Android network. A VPN usually becomes the
//! phone's default network and captures traffic meant for the local network too, where it
//! can't reach the other device. The app names the Wi-Fi (or other local) network by its
//! `Network.getNetworkHandle()`, and the engine connects over that network directly. Only
//! connections to nearby devices are affected; everything else still goes through the VPN.
//!
//! Android only allows this when the VPN lets apps bypass it. When it doesn't, binding is
//! refused (`EPERM`) and the connection takes the default route as before: such a VPN may
//! still leave local traffic alone ("allow LAN"), and if it doesn't, nothing an app does can
//! reach the device, so the app explains that instead.

use std::io;
use std::sync::Arc;

use beam_core::tcp::PrepareSocket;
use tokio::net::TcpSocket;

/// What to do to each socket before it connects: nothing without a network, otherwise send it
/// over `network` (a `Network.getNetworkHandle()`) if Android allows it.
pub(crate) fn over(network: Option<u64>) -> Option<PrepareSocket> {
    network.map(|handle| -> PrepareSocket {
        Arc::new(move |socket| refusal_is_default_route(bind(handle, socket)))
    })
}

/// A VPN that doesn't allow bypassing makes Android refuse the binding; the socket then
/// keeps the default route. Every other failure (an unknown network, say) is an error.
fn refusal_is_default_route(bound: io::Result<()>) -> io::Result<()> {
    match bound {
        Err(error) if error.kind() == io::ErrorKind::PermissionDenied => Ok(()),
        other => other,
    }
}

#[cfg(target_os = "android")]
mod android {
    use std::ffi::c_int;

    #[link(name = "android")]
    #[allow(unsafe_code)] // Declaring a C function; calling it is the unsafe part, see `bind`.
    unsafe extern "C" {
        /// From the NDK (`<android/multinetwork.h>`, API 23+): routes `fd` over `network`.
        /// Returns 0, or -1 with `errno` set.
        pub(super) fn android_setsocknetwork(network: u64, fd: c_int) -> c_int;
    }
}

#[cfg(target_os = "android")]
#[allow(unsafe_code)] // One of the two unsafe blocks in Beam (see docs/security.md).
fn bind(network: u64, socket: &TcpSocket) -> io::Result<()> {
    use std::os::fd::AsRawFd;

    // SAFETY: `android_setsocknetwork` only reads its two integer arguments: it sets a socket
    // option on `fd` and touches no memory of ours. `fd` belongs to `socket`, which the
    // borrow keeps open for the whole call. An unknown network handle is not undefined
    // behaviour: the call fails and sets errno, which is returned as the error.
    let result = unsafe { android::android_setsocknetwork(network, socket.as_raw_fd()) };
    if result == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

#[cfg(not(target_os = "android"))]
fn bind(_network: u64, _socket: &TcpSocket) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "choosing a network is only supported on Android",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_network_leaves_sockets_alone() {
        assert!(over(None).is_none());
    }

    #[test]
    fn a_refused_binding_keeps_the_default_route_and_other_errors_stay() {
        let eperm = io::Error::from_raw_os_error(1);
        assert_eq!(eperm.kind(), io::ErrorKind::PermissionDenied);
        assert!(refusal_is_default_route(Err(eperm)).is_ok());
        assert!(refusal_is_default_route(Ok(())).is_ok());
        let unknown = io::Error::from_raw_os_error(64); // ENONET on Linux and Android
        assert!(refusal_is_default_route(Err(unknown)).is_err());
    }

    #[cfg(not(target_os = "android"))]
    #[tokio::test]
    async fn elsewhere_a_network_is_refused_not_ignored() {
        let prepare = over(Some(42)).unwrap_or_else(|| panic!("a network was given"));
        let socket = TcpSocket::new_v4().unwrap_or_else(|error| panic!("{error}"));
        let error = prepare(&socket)
            .err()
            .unwrap_or_else(|| panic!("it was accepted"));
        assert_eq!(error.kind(), io::ErrorKind::Unsupported);
    }
}
