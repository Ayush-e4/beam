//! The engine object the app creates once: it owns the runtime that every transfer runs on.

use std::collections::VecDeque;
use std::net::{Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, SystemTime};

use beam_core::staging::remove_stale;
use beam_core::tcp::TcpAcceptor;
use beam_core::{Beacon, DiscoveryKey, Sender, TransferId, prepare_opened, prepare_transfer};
use tokio::runtime::{Builder, Runtime};

use crate::incoming::Listener;
use crate::outgoing::OutgoingTransfer;
use crate::pairing::{Local, PairedDevice, PairingHost, pair_with};
use crate::runtime::{blocking, run};
use crate::shared::{SharedFile, duplicate};
use crate::{BeamError, DeviceIdentity, SessionCipher, TrustedKeys, UnfinishedCleanup};

/// How many finished incoming transfers to remember, to answer a sender that missed `done`.
const REMEMBERED_TRANSFERS: usize = 256;

/// Transfers this device received and saved, newest last. Kept in memory for now; the
/// transfer history (later in M2) will make it survive restarts.
#[derive(Debug, Default)]
pub(crate) struct Finished(Mutex<VecDeque<TransferId>>);

impl Finished {
    pub(crate) fn contains(&self, id: TransferId) -> bool {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .contains(&id)
    }

    pub(crate) fn insert(&self, id: TransferId) {
        let mut ids = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        if !ids.contains(&id) {
            if ids.len() == REMEMBERED_TRANSFERS {
                ids.pop_front();
            }
            ids.push_back(id);
        }
    }
}

/// Sends and receives files as this device. Create one per app process.
#[derive(Debug, uniffi::Object)]
pub struct Engine {
    runtime: Option<Runtime>,
    identity: Arc<DeviceIdentity>,
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

    /// Reads the picked files and folders and prepares a transfer. Keep the result and call
    /// [`OutgoingTransfer::send_to`] again after a failure: it resumes where it stopped.
    ///
    /// # Errors
    ///
    /// [`BeamError::Storage`] if a path can't be read or the selection can't be sent.
    pub async fn prepare_send(
        &self,
        paths: Vec<String>,
    ) -> Result<Arc<OutgoingTransfer>, BeamError> {
        let handle = self.handle()?;
        let identity = self.identity.clone();
        let task = handle.clone();
        run(&handle, async move {
            let paths: Vec<PathBuf> = paths.into_iter().map(PathBuf::from).collect();
            let prepared = blocking(move || prepare_transfer(&paths)).await?;
            Ok(OutgoingTransfer::new(task, identity, Sender::new(prepared)))
        })
        .await
    }

    /// Prepares a transfer of files the app has open, such as gallery or share-sheet items.
    /// Each lands at the top of the receiver's folder under its name. The engine takes its
    /// own copy of every descriptor before returning, so the app closes its own afterwards.
    ///
    /// # Errors
    ///
    /// [`BeamError::Storage`] if a descriptor isn't an open regular file or the selection
    /// can't be sent.
    pub async fn prepare_send_shared(
        &self,
        files: Vec<SharedFile>,
    ) -> Result<Arc<OutgoingTransfer>, BeamError> {
        let opened = duplicate(&files)?;
        let handle = self.handle()?;
        let identity = self.identity.clone();
        let task = handle.clone();
        run(&handle, async move {
            let prepared = blocking(move || prepare_opened(opened)).await?;
            Ok(OutgoingTransfer::new(task, identity, Sender::new(prepared)))
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
    /// device that answered isn't the one in the code.
    pub async fn pair_with(&self, code: String, name: String) -> Result<PairedDevice, BeamError> {
        let local = Local {
            identity: self.identity.clone(),
            discovery_key: self.discovery_key(),
        };
        pair_with(&self.handle()?, local, code, name).await
    }
}

impl Engine {
    fn discovery_key(&self) -> DiscoveryKey {
        let generation = self.discovery_generation.load(Ordering::Relaxed);
        DiscoveryKey::derive(self.identity.keypair(), generation)
    }

    fn handle(&self) -> Result<tokio::runtime::Handle, BeamError> {
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
