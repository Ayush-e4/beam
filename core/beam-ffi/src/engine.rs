//! The engine object the app creates once: it owns the runtime that every transfer runs on.

use std::collections::VecDeque;
use std::net::{Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};

use beam_core::tcp::TcpAcceptor;
use beam_core::{Sender, TransferId, prepare_transfer};
use tokio::runtime::{Builder, Runtime};

use crate::incoming::Listener;
use crate::outgoing::OutgoingTransfer;
use crate::runtime::{blocking, run};
use crate::{BeamError, DeviceIdentity, SessionCipher};

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
        }))
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

    /// Listens for transfers from known devices on `port` (0 picks a free one), on every
    /// network interface. Devices connecting must use `cipher`.
    ///
    /// # Errors
    ///
    /// [`BeamError::Network`] if the port can't be opened.
    pub async fn listen(
        &self,
        port: u16,
        cipher: SessionCipher,
    ) -> Result<Arc<Listener>, BeamError> {
        let handle = self.handle()?;
        let identity = self.identity.clone();
        let finished = self.finished.clone();
        let task = handle.clone();
        run(&handle, async move {
            let addr = SocketAddr::from((Ipv4Addr::UNSPECIFIED, port));
            let acceptor = TcpAcceptor::bind(addr).await?;
            Listener::new(task, acceptor, identity, cipher, finished)
        })
        .await
    }
}

impl Engine {
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
