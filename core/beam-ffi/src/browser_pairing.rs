//! Pairing with a browser. The browser shows the code but can't accept connections, so this
//! device scans it, serves WebTransport, and tells the browser where to connect with a sealed
//! offer posted in the code's signaling room. The browser then connects and both pair.

use std::fmt::Write as _;
use std::sync::Arc;
use std::time::Duration;

use beam_core::webtransport::{BrowserStream, WebTransportServer};
use beam_core::{
    Connection, DiscoveryKey, Handshake, Hello, PairingInvite, Role, SignalFrom, SignalMessage,
    SignalRoom, SignalSeal,
};
use tokio::runtime::Handle;
use tokio::sync::Mutex;
use tokio::time::timeout;

use crate::clip_link::{ClipLink, KeepAlive};
use crate::pairing::{PairedDevice, hints, introduce};
use crate::runtime::run;
use crate::{BeamError, DeviceIdentity, NetworkFailure};

/// How long a browser that connected has to finish pairing before the next one gets a turn.
const PAIRING_STEP: Duration = Duration::from_secs(20);

/// How long the signaling service has to take the offer.
const ANNOUNCE_TIME: Duration = Duration::from_secs(15);

/// Serves WebTransport for the browser whose code was scanned, until one browser that holds
/// that code pairs. Single use. The paired connection stays open until [`Self::stop`] or the
/// object is released (`close()` in Kotlin), so what was sent last reaches the browser.
#[derive(uniffi::Object)]
pub struct BrowserPairing {
    handle: Handle,
    server: Arc<WebTransportServer>,
    identity: Arc<DeviceIdentity>,
    discovery_key: DiscoveryKey,
    invite: PairingInvite,
    paired: Arc<Mutex<Option<Connection<BrowserStream>>>>,
    spent: Arc<Mutex<bool>>,
}

impl std::fmt::Debug for BrowserPairing {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BrowserPairing").finish_non_exhaustive()
    }
}

impl BrowserPairing {
    pub(crate) fn new(
        handle: Handle,
        server: WebTransportServer,
        identity: Arc<DeviceIdentity>,
        discovery_key: DiscoveryKey,
        invite: PairingInvite,
    ) -> Arc<Self> {
        Arc::new(Self {
            handle,
            server: Arc::new(server),
            identity,
            discovery_key,
            invite,
            paired: Arc::default(),
            spent: Arc::default(),
        })
    }
}

#[uniffi::export]
impl BrowserPairing {
    /// The port browsers connect to.
    ///
    /// # Errors
    ///
    /// [`BeamError::Network`] if the socket can't report its address.
    pub fn port(&self) -> Result<u16, BeamError> {
        Ok(self.server.local_addr()?.port())
    }

    /// The signaling room to post [`Self::offer`] in: 32 lowercase hex characters, as the
    /// signaling service's `/rooms/{room}` takes it.
    #[must_use]
    pub fn room(&self) -> String {
        self.invite
            .room()
            .iter()
            .fold(String::new(), |mut out, byte| {
                let _ = write!(out, "{byte:02x}"); // writing to a String can't fail
                out
            })
    }

    /// The message to post in the signaling room: this device's local `addresses` (at most 4,
    /// IPv4 first; IPv6 link-local addresses can't be used and are skipped) with the port, and
    /// the hash of the certificate the browser must pin. Sealed so that only the browser
    /// showing the code can open it.
    ///
    /// # Errors
    ///
    /// [`BeamError::Protocol`] if an address can't be parsed or none is left.
    pub fn offer(&self, addresses: Vec<String>) -> Result<Vec<u8>, BeamError> {
        let hints = hints(addresses, self.port()?)?;
        if hints.is_empty() {
            return Err(BeamError::Protocol {
                detail: "no address a browser can connect to".into(),
            });
        }
        let offer = SignalMessage::Offer {
            hints,
            certificate_hash: self.server.certificate_hash(),
        };
        Ok(SignalSeal::for_invite(&self.invite)?.seal(SignalFrom::Scanner, &offer)?)
    }

    /// Posts [`Self::offer`] for `addresses` in the code's room on the signaling `service`
    /// (`wss://…`, or `ws://…` for a local one), where the browser showing the code picks it
    /// up, now or when it joins.
    ///
    /// # Errors
    ///
    /// As [`Self::offer`], and [`BeamError::Network`] if the service can't be reached, turned
    /// this device away or didn't answer in time.
    pub async fn announce(&self, service: String, addresses: Vec<String>) -> Result<(), BeamError> {
        let offer = self.offer(addresses)?;
        let room = self.invite.room();
        run(&self.handle, async move {
            timeout(ANNOUNCE_TIME, SignalRoom::deliver(&service, &room, offer))
                .await
                .map_err(|_| BeamError::Network {
                    detail: "the signaling service didn't answer".into(),
                    failure: NetworkFailure::NoAnswer,
                })??;
            Ok(())
        })
        .await
    }

    /// Waits for the browser showing the code to connect and pair, introducing this device as
    /// `name`. Browsers that fail (another code, too slow) are dropped and it keeps waiting.
    ///
    /// # Errors
    ///
    /// [`BeamError::Protocol`] if a browser already paired here, [`BeamError::Network`] once
    /// closed.
    pub async fn wait_for_browser(&self, name: String) -> Result<PairedDevice, BeamError> {
        let server = self.server.clone();
        let identity = self.identity.clone();
        let invite = self.invite.clone();
        let hello = Hello::new(&name, self.discovery_key.clone());
        let (paired, spent) = (self.paired.clone(), self.spent.clone());
        run(&self.handle, async move {
            let mut spent = spent.lock().await;
            if *spent {
                return Err(BeamError::Protocol {
                    detail: "a browser already paired with this code".into(),
                });
            }
            loop {
                let (stream, _) = server.accept().await?;
                let handshake = Handshake::pairing_scanner(
                    identity.keypair(),
                    invite.cipher,
                    Role::Responder,
                    &invite.token,
                    invite.displayer,
                )?;
                let attempt = async {
                    let mut conn = Connection::establish(stream, handshake).await?;
                    let device = introduce(&mut conn, &hello, invite.cipher.into()).await?;
                    Ok::<_, BeamError>((conn, device))
                };
                if let Ok(Ok((conn, device))) = timeout(PAIRING_STEP, attempt).await {
                    *spent = true;
                    *paired.lock().await = Some(conn);
                    return Ok(device);
                }
            }
        })
        .await
    }

    /// Turns the paired connection into a clipboard link with the browser, which stays open
    /// while both are around, even after this object is released. Single use: no other
    /// browser can connect from then on.
    ///
    /// # Errors
    ///
    /// [`BeamError::Protocol`] if no browser paired yet or the connection was already taken.
    pub async fn link(&self) -> Result<Arc<ClipLink>, BeamError> {
        let conn = self
            .paired
            .lock()
            .await
            .take()
            .ok_or_else(|| BeamError::Protocol {
                detail: "no paired browser connection to link".into(),
            })?;
        self.server.stop_accepting();
        let server: KeepAlive = self.server.clone();
        Ok(ClipLink::start(self.handle.clone(), conn, Some(server)))
    }

    /// Stops serving and closes the paired connection, if any; a pending
    /// [`Self::wait_for_browser`] ends with [`BeamError::Network`]. Not named `close`, which
    /// Kotlin objects already have for releasing them.
    pub fn stop(&self) {
        self.server.close();
    }
}
