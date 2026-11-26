//! The engine object the app creates once: it owns the runtime that every transfer runs on.

use std::net::{Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, SystemTime};

use beam_core::staging::remove_stale;
use beam_core::tcp::TcpAcceptor;
use beam_core::webtransport::WebTransportServer;
use beam_core::{Beacon, DiscoveryKey, PairingInvite};
use tokio::runtime::{Builder, Runtime};

use crate::browser_pairing::BrowserPairing;
use crate::finished::Finished;
use crate::incoming::Listener;
use crate::pairing::{Local, PairedDevice, PairingHost, pair_with};
use crate::runtime::{blocking, run};
use crate::{BeamError, DeviceIdentity, SessionCipher, TrustedKeys, UnfinishedCleanup};

/// Sends and receives files as this device. Create one per app process.
#[derive(Debug, uniffi::Object)]
pub struct Engine {
    runtime: Option<Runtime>,
    pub(crate) identity: Arc<DeviceIdentity>,
    finished: Arc<Finished>,
    discovery_generation: AtomicU32,
}

#[uniffi::export]
impl Engine {
    /// Starts the engine for `identity`.
    ///
    /// # Errors
    ///
    /// [`BeamError::Internal`] if the engine threads cannot start.
    #[uniffi::constructor]
    pub fn new(identity: Arc<DeviceIdentity>) -> Result<Arc<Self>, BeamError> {
        let runtime = Builder::new_multi_thread()
            .thread_name("beam-engine")
            .enable_all()
            .build()
            .map_err(|error| BeamError::Internal {
                detail: format!("could not start the engine: {error}"),
            })?;
        Ok(Arc::new(Self {
            runtime: Some(runtime),
            identity,
            finished: Arc::default(),
            discovery_generation: AtomicU32::new(0),
        }))
    }

    /// Sets the discovery key generation the app keeps (0 until a trusted device is removed;
    /// then one more). Devices paired from now on get the key for this generation.
    pub fn set_discovery_generation(&self, generation: u32) {
        self.discovery_generation
            .store(generation, Ordering::Relaxed);
    }

    /// A new beacon to announce as this device's mDNS service name, for the current discovery
    /// generation. Only trusted devices can tell it is this device; ask for a new one every 15
    /// minutes and whenever the network changes.
    ///
    /// # Errors
    ///
    /// [`BeamError::Crypto`] if the random number generator fails.
    pub fn new_beacon(&self) -> Result<String, BeamError> {
        Ok(Beacon::new(&self.discovery_key())?.encode())
    }

    /// Keeps the list of received transfers in `path`, a file in the app's private storage,
    /// so a sender that asks again after a restart is told the files arrived. Call it once,
    /// before listening.
    ///
    /// # Errors
    ///
    /// [`BeamError::Storage`] if the file can't be read or written, or is damaged.
    pub async fn remember_finished_in(&self, path: String) -> Result<(), BeamError> {
        let handle = self.handle()?;
        let finished = self.finished.clone();
        run(&handle, async move {
            blocking(move || {
                finished
                    .keep_in(PathBuf::from(path))
                    .map_err(beam_core::BeamError::storage)
            })
            .await
        })
        .await
    }

    /// Removes unfinished incoming transfers in `destination` that made no progress for
    /// `max_age_secs`, freeing their space. Senders coming back later start over.
    ///
    /// # Errors
    ///
    /// [`BeamError::Storage`] if `destination` can't be read.
    pub async fn remove_stale_transfers(
        &self,
        destination: String,
        max_age_secs: u64,
    ) -> Result<UnfinishedCleanup, BeamError> {
        let handle = self.handle()?;
        run(&handle, async move {
            let max_age = Duration::from_secs(max_age_secs);
            let path = PathBuf::from(destination);
            blocking(move || remove_stale(&path, max_age, SystemTime::now()))
                .await
                .map(UnfinishedCleanup::from)
        })
        .await
    }

    /// Listens for transfers on `port` (0 picks a free one), on every network interface.
    /// Devices connecting must use `cipher`. Each offer says whether `trusted` knows the
    /// sender; offers from unknown devices are rate-limited per address, and an address whose
    /// offers keep being declined is refused for a while.
    ///
    /// # Errors
    ///
    /// [`BeamError::Network`] if the port can't be opened.
    pub async fn listen(
        &self,
        port: u16,
        cipher: SessionCipher,
        trusted: Arc<dyn TrustedKeys>,
    ) -> Result<Arc<Listener>, BeamError> {
        let handle = self.handle()?;
        let identity = self.identity.clone();
        let finished = self.finished.clone();
        let task = handle.clone();
        run(&handle, async move {
            let addr = SocketAddr::from((Ipv4Addr::UNSPECIFIED, port));
            let acceptor = TcpAcceptor::bind(addr).await?;
            Listener::new(task, acceptor, identity, cipher, finished, trusted)
        })
        .await
    }
}

#[uniffi::export]
impl Engine {
    /// Starts showing a pairing code: listens on a free port on every interface. Use
    /// [`PairingHost::code`] for the QR text and [`PairingHost::wait_for_device`] to pair.
    ///
    /// # Errors
    ///
    /// [`BeamError::Network`] if no port can be opened.
    pub async fn host_pairing(&self, cipher: SessionCipher) -> Result<Arc<PairingHost>, BeamError> {
        let handle = self.handle()?;
        let identity = self.identity.clone();
        let discovery_key = self.discovery_key();
        let task = handle.clone();
        run(&handle, async move {
            let acceptor = TcpAcceptor::bind(SocketAddr::from((Ipv4Addr::UNSPECIFIED, 0))).await?;
            PairingHost::new(task, acceptor, identity, discovery_key, cipher)
        })
        .await
    }

    /// Pairs with the device showing the scanned `code`, introducing this one as `name`.
    ///
    /// # Errors
    ///
    /// [`BeamError::Protocol`] if `code` isn't a Beam pairing code (the message says why),
    /// [`BeamError::Network`] if the device can't be reached, and [`BeamError::Crypto`] if the
    /// device that answered isn't the one in the code. `network` is as for
    /// [`crate::OutgoingTransfer::send_to`].
    #[uniffi::method(default(network = None))]
    pub async fn pair_with(
        &self,
        code: String,
        name: String,
        network: Option<u64>,
    ) -> Result<PairedDevice, BeamError> {
        let local = Local {
            identity: self.identity.clone(),
            discovery_key: self.discovery_key(),
        };
        pair_with(&self.handle()?, local, code, name, network).await
    }
}

#[uniffi::export]
impl Engine {
    /// Pairs with the browser showing the scanned `code`: serves WebTransport on a free port
    /// on every interface. Post [`BrowserPairing::offer`] in [`BrowserPairing::room`], then
    /// [`BrowserPairing::wait_for_browser`].
    ///
    /// # Errors
    ///
    /// [`BeamError::Protocol`] if `code` isn't a Beam pairing code, [`BeamError::Network`] if
    /// no port can be opened, [`BeamError::Crypto`] if the server's certificate can't be made.
    pub async fn pair_with_browser(&self, code: String) -> Result<Arc<BrowserPairing>, BeamError> {
        let invite = PairingInvite::decode(&code)?;
        let handle = self.handle()?;
        let identity = self.identity.clone();
        let discovery_key = self.discovery_key();
        let task = handle.clone();
        run(&handle, async move {
            let server = WebTransportServer::bind(SocketAddr::from((Ipv4Addr::UNSPECIFIED, 0)))?;
            Ok(BrowserPairing::new(
                task,
                server,
                identity,
                discovery_key,
                invite,
            ))
        })
        .await
    }
}

impl Engine {
    fn discovery_key(&self) -> DiscoveryKey {
        let generation = self.discovery_generation.load(Ordering::Relaxed);
        DiscoveryKey::derive(self.identity.keypair(), generation)
    }

    pub(crate) fn handle(&self) -> Result<tokio::runtime::Handle, BeamError> {
        self.runtime
            .as_ref()
            .map(|runtime| runtime.handle().clone())
            .ok_or_else(|| BeamError::Cancelled {
                detail: "the engine shut down".into(),
            })
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        // Safe from any thread, including an engine thread: running transfers are abandoned
        // and resume from their journals next time.
        if let Some(runtime) = self.runtime.take() {
            runtime.shutdown_background();
        }
    }
}
