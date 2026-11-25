//! Receiving: a listener that yields offers, which the app accepts or declines.

use std::net::IpAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use beam_core::staging::continues;
use beam_core::tcp::TcpAcceptor;
use beam_core::{Connection, Manifest, Receiver, receive_transfer, reject_offer};
use tokio::net::TcpStream;
use tokio::runtime::Handle;
use tokio::sync::mpsc;
use tokio::task::AbortHandle;

use crate::engine::Finished;
use crate::gate::{SenderGate, lock};
use crate::inbox::{Inbox, QUEUED_OFFERS, Queued};
use crate::progress::{ProgressListener, reporting};
use crate::runtime::{blocking, run};
use crate::{
    BeamError, DeclineReason, DeviceIdentity, SessionCipher, TransferSummary, TrustedKeys,
};

type Link = Connection<TcpStream>;

/// Accepts connections from other devices in the background, many at once, and hands out
/// their offers. Close it to stop listening; offers not yet handed out are dropped.
#[derive(uniffi::Object)]
pub struct Listener {
    handle: Handle,
    port: u16,
    offers: Arc<tokio::sync::Mutex<mpsc::Receiver<Queued>>>,
    accepting: AbortHandle,
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
        let (queue, offers) = mpsc::channel(QUEUED_OFFERS);
        let inbox = Arc::new(Inbox {
            handle: handle.clone(),
            acceptor,
            identity,
            cipher: cipher.into(),
            finished,
            trusted,
            gate: Arc::default(),
        });
        let accepting = handle.spawn(inbox.run(queue)).abort_handle();
        Ok(Arc::new(Self {
            handle,
            port,
            offers: Arc::new(tokio::sync::Mutex::new(offers)),
            accepting,
        }))
    }
}

impl Drop for Listener {
    fn drop(&mut self) {
        self.accepting.abort();
    }
}

#[uniffi::export]
impl Listener {
    /// The port it listens on.
    #[must_use]
    pub fn port(&self) -> u16 {
        self.port
    }

    /// Waits for the next offer. Devices are handled side by side, so one that connects and
    /// stays silent can't hold up the others (at most 4 connections in progress per address,
    /// 32 in total). Connections that fail the handshake or never send a valid offer are
    /// dropped; a repeated offer for a transfer already saved is confirmed automatically, so
    /// the sender finishes without sending it twice. Unknown devices over their limit are
    /// turned away as busy without reaching the app, and blocked addresses are cut off before
    /// the handshake. Cancelling the call only stops waiting; the listener keeps accepting.
    ///
    /// # Errors
    ///
    /// [`BeamError::Crypto`] only if this device's own key is unusable.
    pub async fn next_offer(&self) -> Result<Arc<IncomingOffer>, BeamError> {
        let offers = self.offers.clone();
        run(&self.handle, async move {
            offers.lock().await.recv().await.unwrap_or_else(|| {
                Err(BeamError::Cancelled {
                    detail: "the listener stopped".into(),
                })
            })
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
pub(crate) struct Sender {
    pub(crate) ip: IpAddr,
    pub(crate) trusted: bool,
    pub(crate) gate: Arc<Mutex<SenderGate>>,
}

impl std::fmt::Debug for IncomingOffer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IncomingOffer")
            .field("peer", &self.peer)
            .finish_non_exhaustive()
    }
}

impl IncomingOffer {
    pub(crate) fn new(
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

    /// Whether this offer continues a transfer from the same device that was already accepted
    /// into `destination` and is unfinished there (for example, cut off when the app was
    /// closed). The app can accept such an offer without asking again.
    ///
    /// # Errors
    ///
    /// [`BeamError::Internal`] if the engine can't check.
    pub async fn continues_in(&self, destination: String) -> Result<bool, BeamError> {
        let transfer_id = self.manifest.transfer_id();
        let sender = self.peer_public_key();
        run(&self.handle, async move {
            blocking(move || Ok(continues(&PathBuf::from(destination), transfer_id, &sender))).await
        })
        .await
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
        let sender = self.peer_public_key();
        run(&self.handle, async move {
            let mut conn = take(&conn).await?;
            let transfer_id = manifest.transfer_id();
            let receiver = blocking(move || {
                Receiver::open_from(manifest, &PathBuf::from(destination), &selection, &sender)
            })
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
