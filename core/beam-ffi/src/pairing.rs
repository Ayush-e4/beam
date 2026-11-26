//! Pairing: one device shows a code, the other scans it, and they learn each other's keys.

use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;

use beam_core::tcp::{self, TcpAcceptor};
use beam_core::{Connection, DiscoveryKey, Handshake, Hello, PairingInvite, Role, exchange_hello};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::TcpStream;
use tokio::runtime::Handle;
use tokio::task::JoinSet;

use crate::network::over;
use crate::runtime::run;
use crate::{BeamError, DeviceIdentity, SessionCipher};

/// A device this one just paired with. Store it as a trusted device.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct PairedDevice {
    /// Its public key, which later connections must prove.
    pub public_key: Vec<u8>,
    /// Its device ID.
    pub device_id: String,
    /// The name it gave, for showing in the trusted-device list.
    pub name: String,
    /// The cipher chosen when pairing; use it for every later connection.
    pub cipher: SessionCipher,
    /// Six digits both screens can show, so people can confirm nobody sat in between.
    pub verification_code: String,
    /// The key its discovery beacons are made with. Secret: store it with the device.
    pub discovery_key: Vec<u8>,
}

/// This device, as pairing introduces it.
#[derive(Debug, Clone)]
pub(crate) struct Local {
    pub(crate) identity: Arc<DeviceIdentity>,
    pub(crate) discovery_key: DiscoveryKey,
}

/// Shows a pairing code and waits for one device to scan it and connect. Single use: once a
/// device has paired, the code is spent. Close it (or cancel the wait) to withdraw the code.
#[derive(uniffi::Object)]
pub struct PairingHost {
    handle: Handle,
    acceptor: Arc<TcpAcceptor>,
    identity: Arc<DeviceIdentity>,
    discovery_key: DiscoveryKey,
    invite: PairingInvite,
    spent: Arc<tokio::sync::Mutex<bool>>,
}

impl std::fmt::Debug for PairingHost {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PairingHost").finish_non_exhaustive()
    }
}

impl PairingHost {
    pub(crate) fn new(
        handle: Handle,
        acceptor: TcpAcceptor,
        identity: Arc<DeviceIdentity>,
        discovery_key: DiscoveryKey,
        cipher: SessionCipher,
    ) -> Result<Arc<Self>, BeamError> {
        let invite = PairingInvite::new(identity.keypair().public(), cipher.into(), Vec::new())?;
        Ok(Arc::new(Self {
            handle,
            acceptor: Arc::new(acceptor),
            identity,
            discovery_key,
            invite,
            spent: Arc::default(),
        }))
    }
}

#[uniffi::export]
impl PairingHost {
    /// The port it accepts connections on.
    ///
    /// # Errors
    ///
    /// [`BeamError::Network`] if the socket can't report its address.
    pub fn port(&self) -> Result<u16, BeamError> {
        Ok(self.acceptor.local_addr()?.port())
    }

    /// The text to show as a QR code, telling the scanner to try these local `addresses`
    /// (at most 4, IPv4 first; IPv6 link-local addresses can't be used and are skipped).
    ///
    /// # Errors
    ///
    /// [`BeamError::Protocol`] if an address can't be parsed.
    pub fn code(&self, addresses: Vec<String>) -> Result<String, BeamError> {
        let hints = hints(addresses, self.port()?)?;
        Ok(self.invite.with_hints(hints)?.encode())
    }

    /// Waits for a device to scan the code and pair, introducing this device as `name`.
    /// Devices that fail (an old code, someone else's key) are ignored and it keeps waiting.
    ///
    /// # Errors
    ///
    /// [`BeamError::Protocol`] if this code already paired a device.
    pub async fn wait_for_device(&self, name: String) -> Result<PairedDevice, BeamError> {
        let acceptor = self.acceptor.clone();
        let identity = self.identity.clone();
        let invite = self.invite.clone();
        let hello = Hello::new(&name, self.discovery_key.clone());
        let spent = self.spent.clone();
        run(&self.handle, async move {
            let mut spent = spent.lock().await;
            if *spent {
                return Err(BeamError::Protocol {
                    detail: "this pairing code was already used".into(),
                });
            }
            loop {
                let handshake = Handshake::pairing_displayer(
                    identity.keypair(),
                    invite.cipher,
                    Role::Responder,
                    &invite.token,
                )?;
                let Ok((mut conn, _)) = acceptor.accept(handshake).await else {
                    continue; // wrong token, or not a Beam device
                };
                if let Ok(device) = introduce(&mut conn, &hello, invite.cipher.into()).await {
                    *spent = true;
                    return Ok(device);
                }
            }
        })
        .await
    }
}

/// Scans `code` and pairs with the device showing it, introducing this device as `name`.
pub(crate) async fn pair_with(
    handle: &Handle,
    local: Local,
    code: String,
    name: String,
    network: Option<u64>,
) -> Result<PairedDevice, BeamError> {
    let invite = PairingInvite::decode(&code)?;
    if invite.hints.is_empty() {
        return Err(BeamError::network("this code has no address to connect to"));
    }
    run(handle, async move {
        let mut conn = connect_first(&local.identity, &invite, network).await?;
        let hello = Hello::new(&name, local.discovery_key);
        introduce(&mut conn, &hello, invite.cipher.into()).await
    })
    .await
}

/// Tries every hint at once and keeps the first device that completes the handshake.
async fn connect_first(
    identity: &Arc<DeviceIdentity>,
    invite: &PairingInvite,
    network: Option<u64>,
) -> Result<Connection<TcpStream>, BeamError> {
    let mut attempts = JoinSet::new();
    for &hint in &invite.hints {
        let handshake = Handshake::pairing_scanner(
            identity.keypair(),
            invite.cipher,
            Role::Initiator,
            &invite.token,
            invite.displayer,
        )?;
        attempts.spawn(tcp::connect_prepared(hint, handshake, over(network)));
    }
    let mut last_error = None;
    while let Some(attempt) = attempts.join_next().await {
        match attempt {
            Ok(Ok(conn)) => return Ok(conn), // dropping the set aborts the others
            Ok(Err(error)) => last_error = Some(error.into()),
            Err(error) => {
                last_error = Some(BeamError::Internal {
                    detail: format!("connect task failed: {error}"),
                });
            }
        }
    }
    Err(last_error.unwrap_or_else(|| BeamError::network("no address to connect to")))
}

/// `addresses` with `port`, as connection hints: at most 4, skipping IPv6 link-local addresses
/// (they need an interface to be usable).
pub(crate) fn hints(addresses: Vec<String>, port: u16) -> Result<Vec<SocketAddr>, BeamError> {
    let mut hints = Vec::new();
    for address in addresses {
        let ip: IpAddr = address.parse().map_err(|_| BeamError::Protocol {
            detail: format!("{address} is not an IP address"),
        })?;
        if !is_link_local_v6(ip) && hints.len() < beam_core::invite::MAX_HINTS {
            hints.push(SocketAddr::new(ip, port));
        }
    }
    Ok(hints)
}

/// Swaps `HELLO`s right after a pairing handshake and describes the device on the other end.
pub(crate) async fn introduce<S: AsyncRead + AsyncWrite + Unpin>(
    conn: &mut Connection<S>,
    hello: &Hello,
    cipher: SessionCipher,
) -> Result<PairedDevice, BeamError> {
    let peer = exchange_hello(conn, hello).await?;
    let key = conn.peer();
    Ok(PairedDevice {
        public_key: key.as_bytes().to_vec(),
        device_id: key.device_id().to_string(),
        name: peer.name,
        cipher,
        verification_code: conn.verification_code().to_string(),
        discovery_key: peer.discovery_key.as_bytes().to_vec(),
    })
}

fn is_link_local_v6(ip: IpAddr) -> bool {
    matches!(ip, IpAddr::V6(v6) if (v6.segments()[0] & 0xffc0) == 0xfe80)
}
