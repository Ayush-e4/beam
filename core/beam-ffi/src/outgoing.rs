//! Sending: one prepared transfer, sent (and resent after failures) to one device.

use std::sync::{Arc, Mutex, PoisonError};

use beam_core::{Handshake, Sender, send_transfer, tcp};
use tokio::net::lookup_host;
use tokio::runtime::Handle;

use crate::network::over;
use crate::progress::{ProgressListener, reporting};
use crate::runtime::run;
use crate::types::peer_key;
use crate::{BeamError, DeviceIdentity, SendResult, SessionCipher, TransferSummary};

/// Files ready to send. Its ID stays the same for every attempt, so the receiver resumes.
#[derive(uniffi::Object)]
pub struct OutgoingTransfer {
    handle: Handle,
    identity: Arc<DeviceIdentity>,
    sender: Arc<Mutex<Sender>>,
    summary: TransferSummary,
    /// One attempt at a time; a second call waits for the first to end.
    attempt: Arc<tokio::sync::Mutex<()>>,
}

impl std::fmt::Debug for OutgoingTransfer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OutgoingTransfer")
            .field("transfer_id", &self.summary.transfer_id)
            .finish_non_exhaustive()
    }
}

impl OutgoingTransfer {
    pub(crate) fn new(handle: Handle, identity: Arc<DeviceIdentity>, sender: Sender) -> Arc<Self> {
        let summary = TransferSummary::from(sender.manifest());
        Arc::new(Self {
            handle,
            identity,
            sender: Arc::new(Mutex::new(sender)),
            summary,
            attempt: Arc::default(),
        })
    }
}

#[uniffi::export]
impl OutgoingTransfer {
    /// The files being sent.
    #[must_use]
    pub fn summary(&self) -> TransferSummary {
        self.summary.clone()
    }

    /// Connects to the device at `host`:`port`, which must prove it owns `peer_public_key`,
    /// and sends. Cancelling the call drops the connection; calling again resumes.
    ///
    /// # Errors
    ///
    /// [`BeamError::Network`] if the device is unreachable, doesn't answer within 10 s, or
    /// the link drops (worth retrying). A device without `peer_public_key` can't read the
    /// handshake and hangs up, which also shows as a network error. [`BeamError::Storage`] if
    /// a file changed or can't be read, [`BeamError::Cancelled`] if the receiver cancelled.
    /// `network` (Android's `Network.getNetworkHandle()`) sends the connection over that
    /// network instead of the default one, which may be a VPN that can't reach the device;
    /// elsewhere it is a network error.
    #[uniffi::method(default(network = None))]
    pub async fn send_to(
        &self,
        host: String,
        port: u16,
        peer_public_key: Vec<u8>,
        cipher: SessionCipher,
        progress: Arc<dyn ProgressListener>,
        network: Option<u64>,
    ) -> Result<SendResult, BeamError> {
        let peer = peer_key(&peer_public_key)?;
        let identity = self.identity.clone();
        let sender = self.sender.clone();
        let attempt = self.attempt.clone();
        run(&self.handle, async move {
            let _one_at_a_time = attempt.lock().await;
            let addr = lookup_host((host.as_str(), port))
                .await
                .map_err(beam_core::BeamError::network)?
                .next()
                .ok_or_else(|| BeamError::network(format!("{host} has no address")))?;
            let handshake = Handshake::known_initiator(identity.keypair(), cipher.into(), peer)?;
            let watched = sender
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .progress();
            // Boxed: the transfer's state is large, and callers hold this future.
            let outcome = Box::pin(reporting(watched, progress, async {
                let mut conn = tcp::connect_prepared(addr, handshake, over(network)).await?;
                send_transfer(&mut conn, &sender).await
            }))
            .await?;
            Ok(outcome.into())
        })
        .await
    }
}
