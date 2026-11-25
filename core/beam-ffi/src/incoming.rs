//! Receiving: a listener that yields offers, which the app accepts or declines.

use std::net::IpAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use beam_core::tcp::TcpAcceptor;
use beam_core::{
    Connection, Handshake, Manifest, Receiver, confirm_received, read_offer, receive_transfer,
    reject_offer,
};
use tokio::net::TcpStream;
use tokio::runtime::Handle;
use tokio::time::timeout;

use crate::engine::Finished;
use crate::gate::SenderGate;
use crate::progress::{ProgressListener, reporting};
use crate::runtime::{blocking, run};
use crate::{
    BeamError, DeclineReason, DeviceIdentity, SessionCipher, TransferSummary, TrustedKeys,
};

/// How long a connected device has to send its offer.
const OFFER_TIMEOUT: Duration = Duration::from_secs(30);

/// Pause after a failed incoming connection.
const RETRY_PAUSE: Duration = Duration::from_millis(50);

type Link = Connection<TcpStream>;

/// Accepts connections from other devices. Close it to stop listening.
#[derive(uniffi::Object)]
pub struct Listener {
    handle: Handle,
    acceptor: Arc<TcpAcceptor>,
    port: u16,
    identity: Arc<DeviceIdentity>,
    cipher: SessionCipher,
    finished: Arc<Finished>,
    trusted: Arc<dyn TrustedKeys>,
    gate: Arc<Mutex<SenderGate>>,
}

impl std::fmt::Debug for Listener {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Listener")
            .field("port", &self.port)
            .finish_non_exhaustive()
    }
}

impl Listener {
    pub(crate) fn new(
        handle: Handle,
        acceptor: TcpAcceptor,
        identity: Arc<DeviceIdentity>,
        cipher: SessionCipher,
        finished: Arc<Finished>,
        trusted: Arc<dyn TrustedKeys>,
    ) -> Result<Arc<Self>, BeamError> {
        let port = acceptor.local_addr()?.port();
        Ok(Arc::new(Self {
            handle,
            acceptor: Arc::new(acceptor),
            port,
            identity,
            cipher,
            finished,
            trusted,
            gate: Arc::default(),
        }))
    }
}

#[uniffi::export]
impl Listener {
    /// The port it listens on.
    #[must_use]
    pub fn port(&self) -> u16 {
        self.port
    }

    /// Waits for the next offer. Connections that fail the handshake or never send a valid
    /// offer are dropped and it keeps waiting; a repeated offer for a transfer already saved is
    /// confirmed automatically, so the sender finishes without sending it twice. Unknown
    /// devices over their limit are turned away as busy without reaching the app, and blocked
    /// addresses are cut off before the handshake. Cancel the call (or close the listener) to
    /// stop waiting.
    ///
    /// # Errors
    ///
    /// [`BeamError::Crypto`] only if this device's own key is unusable.
    pub async fn next_offer(&self) -> Result<Arc<IncomingOffer>, BeamError> {
        let acceptor = self.acceptor.clone();
        let identity = self.identity.clone();
        let cipher = self.cipher.into();
        let finished = self.finished.clone();
        let trusted = self.trusted.clone();
        let gate = self.gate.clone();
        let handle = self.handle.clone();
        run(&self.handle, async move {
            loop {
                let handshake = Handshake::known_responder(identity.keypair(), cipher)?;
                let allowed =
                    |from: std::net::SocketAddr| !lock(&gate).is_blocked(from.ip(), Instant::now());
                let Ok((mut conn, from)) = acceptor.accept_from(handshake, allowed).await else {
                    // A device failed the handshake or gave up. One bad device must not end
                    // listening; the pause keeps a broken socket from spinning.
                    tokio::time::sleep(RETRY_PAUSE).await;
                    continue;
                };
                let Ok(Ok(manifest)) = timeout(OFFER_TIMEOUT, read_offer(&mut conn)).await else {
                    continue; // no valid offer in time
                };
                if finished.contains(manifest.transfer_id()) {
                    // Best effort: if it fails, the sender asks again on its next attempt.
                    let _ = confirm_received(&mut conn, &manifest).await;
                    continue;
                }
                let is_trusted = trusted.is_trusted(conn.peer().as_bytes().to_vec());
                if !is_trusted && !lock(&gate).admit_unknown_offer(from.ip(), Instant::now()) {
                    // Best effort: a sender over its limit learns it should wait.
                    let _ = reject_offer(
                        &mut conn,
                        manifest.transfer_id(),
                        DeclineReason::Busy.into(),
                    )
                    .await;
                    continue;
                }
                let sender = Sender {
                    ip: from.ip(),
                    trusted: is_trusted,
                    gate: gate.clone(),
                };
                return Ok(IncomingOffer::new(handle, conn, manifest, finished, sender));
            }
        })
        .await
    }
}

/// A transfer another device offered. Answer it once, with accept or decline.
#[derive(uniffi::Object)]
pub struct IncomingOffer {
    handle: Handle,
    conn: Arc<tokio::sync::Mutex<Option<Link>>>,
    manifest: Manifest,
    peer: beam_core::PublicKey,
    finished: Arc<Finished>,
    sender: Sender,
}

/// Where an offer came from, for the abuse limits.
struct Sender {
    ip: IpAddr,
    trusted: bool,
    gate: Arc<Mutex<SenderGate>>,
}

fn lock(gate: &Mutex<SenderGate>) -> std::sync::MutexGuard<'_, SenderGate> {
    gate.lock().unwrap_or_else(PoisonError::into_inner)
}

impl std::fmt::Debug for IncomingOffer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IncomingOffer")
            .field("peer", &self.peer)
            .finish_non_exhaustive()
    }
}

impl IncomingOffer {
    fn new(
        handle: Handle,
        conn: Link,
        manifest: Manifest,
        finished: Arc<Finished>,
        sender: Sender,
    ) -> Arc<Self> {
        Arc::new(Self {
            handle,
            peer: conn.peer(),
            conn: Arc::new(tokio::sync::Mutex::new(Some(conn))),
            manifest,
            finished,
            sender,
        })
    }
}

#[uniffi::export]
impl IncomingOffer {
    /// The files offered.
    #[must_use]
    pub fn summary(&self) -> TransferSummary {
        TransferSummary::from(&self.manifest)
    }

    /// The sender's authenticated public key; check it against trusted devices before accepting.
    #[must_use]
    pub fn peer_public_key(&self) -> Vec<u8> {
        self.peer.as_bytes().to_vec()
    }

    /// Whether the sender is a trusted device. Offers from anyone else must be shown to the
    /// person and only accepted if they say yes.
    #[must_use]
    pub fn from_trusted_device(&self) -> bool {
        self.sender.trusted
    }

    /// The sender's device ID.
    #[must_use]
    pub fn peer_device_id(&self) -> String {
        self.peer.device_id().to_string()
    }

    /// Receives the offered files into `destination` (an existing folder); `selection` lists
    /// item IDs to take, empty for all. Returns the saved paths. If it fails or is cancelled,
    /// the sender's next attempt resumes into the same folder.
    ///
    /// # Errors
    ///
    /// [`BeamError::Network`] if the link drops, [`BeamError::Storage`] if saving fails,
    /// [`BeamError::Cancelled`] if the sender cancelled, [`BeamError::Protocol`] if this offer
    /// was already answered.
    pub async fn accept(
        &self,
        destination: String,
        selection: Vec<u32>,
        progress: Arc<dyn ProgressListener>,
    ) -> Result<Vec<String>, BeamError> {
        let conn = self.conn.clone();
        let manifest = self.manifest.clone();
        let finished = self.finished.clone();
        run(&self.handle, async move {
            let mut conn = take(&conn).await?;
            let transfer_id = manifest.transfer_id();
            let receiver =
                blocking(move || Receiver::open(manifest, &PathBuf::from(destination), &selection))
                    .await?;
            let watched = receiver.progress();
            let saved = reporting(watched, progress, receive_transfer(&mut conn, receiver)).await?;
            finished.insert(transfer_id);
            Ok(saved
                .into_iter()
                .map(|path| path.to_string_lossy().into_owned())
                .collect())
        })
        .await
    }

    /// Declines the offer and tells the sender why. Declining an unknown device counts
    /// towards blocking its address.
    ///
    /// # Errors
    ///
    /// [`BeamError::Network`] if the answer can't be sent, [`BeamError::Protocol`] if this
    /// offer was already answered.
    pub async fn decline(&self, reason: DeclineReason) -> Result<(), BeamError> {
        let conn = self.conn.clone();
        let transfer_id = self.manifest.transfer_id();
        let counts = !self.sender.trusted && reason == DeclineReason::UserDeclined;
        let (gate, ip) = (self.sender.gate.clone(), self.sender.ip);
        run(&self.handle, async move {
            let mut conn = take(&conn).await?;
            if counts {
                lock(&gate).declined(ip, Instant::now());
            }
            Ok(reject_offer(&mut conn, transfer_id, reason.into()).await?)
        })
        .await
    }
}

async fn take(conn: &tokio::sync::Mutex<Option<Link>>) -> Result<Link, BeamError> {
    conn.lock().await.take().ok_or_else(|| BeamError::Protocol {
        detail: "this offer was already answered".into(),
    })
}
