//! Meeting a browser this device already trusts, without a pairing code (see `protocol.md`,
//! Meeting again). The phone waits in the pair's signaling room and posts a fresh offer each
//! time the browser is there; the browser connects with Noise IK for the phone's key, and the
//! phone accepts only that browser's key.

use std::convert::Infallible;
use std::sync::Arc;
use std::time::Duration;

use beam_core::webtransport::{BrowserStream, WebTransportServer};
use beam_core::{
    Cipher, Connection, Handshake, Meeting, PublicKey, RoomEvent, SignalFrom, SignalMessage,
    SignalRoom,
};
use tokio::runtime::Handle;
use tokio::sync::Mutex;
use tokio::time::{sleep, timeout};

use crate::clip_link::{ClipLink, KeepAlive};
use crate::pairing::hints;
use crate::runtime::run;
use crate::{BeamError, DeviceIdentity};

/// How long a browser that connected has to finish the handshake.
const HANDSHAKE_TIME: Duration = Duration::from_secs(20);
/// The wait before joining the room again.
const FIRST_RETRY: Duration = Duration::from_secs(1);
/// The longest wait before joining the room again while the service fails.
const MAX_RETRY: Duration = Duration::from_secs(30);

/// Waits for one trusted browser and links its clipboard. Single use; cancel the call to
/// [`Self::meet`] (or [`Self::stop`]) to stop waiting.
#[derive(uniffi::Object)]
pub struct BrowserMeeting {
    handle: Handle,
    server: Arc<WebTransportServer>,
    identity: Arc<DeviceIdentity>,
    browser: PublicKey,
    cipher: Cipher,
    service: String,
    offer: Vec<u8>,
    room: [u8; 16],
    spent: Arc<Mutex<bool>>,
}

impl std::fmt::Debug for BrowserMeeting {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BrowserMeeting").finish_non_exhaustive()
    }
}

/// What [`BrowserMeeting::new`] needs.
pub(crate) struct MeetingSetup {
    pub handle: Handle,
    pub server: WebTransportServer,
    pub identity: Arc<DeviceIdentity>,
    pub browser: PublicKey,
    pub cipher: Cipher,
    pub service: String,
    pub addresses: Vec<String>,
}

impl BrowserMeeting {
    /// Seals the offer once: the server, its certificate and the addresses don't change.
    pub(crate) fn new(setup: MeetingSetup) -> Result<Arc<Self>, BeamError> {
        let meeting = Meeting::between(setup.identity.keypair(), &setup.browser)?;
        let hints = hints(setup.addresses, setup.server.local_addr()?.port())?;
        if hints.is_empty() {
            return Err(BeamError::Protocol {
                detail: "no address a browser can connect to".into(),
            });
        }
        let offer = SignalMessage::Offer {
            hints,
            certificate_hash: setup.server.certificate_hash(),
        };
        let offer = meeting.seal().seal(SignalFrom::Scanner, &offer)?;
        Ok(Arc::new(Self {
            handle: setup.handle,
            server: Arc::new(setup.server),
            identity: setup.identity,
            browser: setup.browser,
            cipher: setup.cipher,
            service: setup.service,
            offer,
            room: meeting.room(),
            spent: Arc::default(),
        }))
    }
}

#[uniffi::export]
impl BrowserMeeting {
    /// Waits in the room until the browser connects, then links its clipboard. Keeps waiting
    /// through the room expiring and the signaling service failing (joining again at least
    /// every 30 seconds); other devices that connect are dropped.
    ///
    /// # Errors
    ///
    /// [`BeamError::Protocol`] if this meeting already linked, [`BeamError::Network`] once
    /// stopped.
    pub async fn meet(&self) -> Result<Arc<ClipLink>, BeamError> {
        let (server, identity) = (self.server.clone(), self.identity.clone());
        let (browser, cipher, spent) = (self.browser, self.cipher, self.spent.clone());
        let (service, room, offer) = (self.service.clone(), self.room, self.offer.clone());
        let handle = self.handle.clone();
        run(&self.handle, async move {
            let mut spent = spent.lock().await;
            if *spent {
                return Err(BeamError::Protocol {
                    detail: "this meeting already linked".into(),
                });
            }
            let conn = tokio::select! {
                conn = accept(&server, &identity, cipher, browser) => conn?,
                never = wait_in_room(&service, room, &offer) => match never {},
            };
            *spent = true;
            server.stop_accepting();
            let keep: KeepAlive = server;
            Ok(ClipLink::start(handle, conn, Some(keep)))
        })
        .await
    }

    /// Stops waiting: a pending [`Self::meet`] ends with [`BeamError::Network`]. Leaves a link
    /// already made alone.
    pub fn stop(&self) {
        self.server.stop_accepting();
    }
}

/// The first connection that completes Noise IK as `browser`.
async fn accept(
    server: &WebTransportServer,
    identity: &DeviceIdentity,
    cipher: Cipher,
    browser: PublicKey,
) -> Result<Connection<BrowserStream>, BeamError> {
    loop {
        let (stream, _) = server.accept().await?;
        let handshake = Handshake::known_responder(identity.keypair(), cipher)?;
        if let Ok(Ok(conn)) =
            timeout(HANDSHAKE_TIME, Connection::establish(stream, handshake)).await
            && conn.peer() == browser
        {
            return Ok(conn);
        }
    }
}

/// Stays in the room for good, posting `offer` whenever the browser is there. After the room
/// closes (it expires every few minutes) it joins again a second later; while the service
/// fails, it waits longer each time, up to [`MAX_RETRY`].
async fn wait_in_room(service: &str, room: [u8; 16], offer: &[u8]) -> Infallible {
    let mut retry = FIRST_RETRY;
    loop {
        if visit(service, room, offer).await.is_ok() {
            retry = FIRST_RETRY;
        }
        sleep(retry).await;
        retry = (retry * 2).min(MAX_RETRY);
    }
}

/// One stay in the room, until it closes.
async fn visit(service: &str, room: [u8; 16], offer: &[u8]) -> Result<(), BeamError> {
    let mut joined = SignalRoom::join(service, &room).await?;
    while let Some(event) = joined.next().await? {
        let browser_here = match event {
            RoomEvent::Joined { peers } => peers > 0,
            RoomEvent::PeerJoined => true,
            RoomEvent::PeerLeft | RoomEvent::BufferFull | RoomEvent::Message(_) => false,
        };
        if browser_here {
            joined.post(offer.to_vec()).await?;
        }
    }
    Ok(())
}
