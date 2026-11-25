//! Preparing transfers to send: from paths, or from files the app has open. A send cut off
//! when the app was closed is prepared again under its old transfer ID, so the receiver
//! continues it instead of starting over.

use std::path::PathBuf;
use std::sync::Arc;

use beam_core::{
    OpenedFile, PreparedTransfer, Sender, prepare_opened, prepare_opened_with_id, prepare_transfer,
    prepare_transfer_with_id,
};

use crate::BeamError;
use crate::engine::Engine;
use crate::outgoing::OutgoingTransfer;
use crate::runtime::{blocking, run};
use crate::shared::{SharedFile, duplicate};
use crate::types::parse_transfer_id;

#[uniffi::export]
impl Engine {
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
        let paths = to_paths(paths);
        self.prepare(move || prepare_transfer(&paths)).await
    }

    /// Like [`Engine::prepare_send`], under the earlier `transfer_id` (from
    /// [`crate::TransferSummary`]) of a send cut off when the app was closed.
    ///
    /// # Errors
    ///
    /// As for [`Engine::prepare_send`]; [`BeamError::Internal`] if `transfer_id` isn't one.
    pub async fn resume_send(
        &self,
        paths: Vec<String>,
        transfer_id: String,
    ) -> Result<Arc<OutgoingTransfer>, BeamError> {
        let id = parse_transfer_id(&transfer_id)?;
        let paths = to_paths(paths);
        self.prepare(move || prepare_transfer_with_id(&paths, id))
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
        self.prepare(move || prepare_opened(opened)).await
    }

    /// Like [`Engine::prepare_send_shared`], under the earlier `transfer_id` of a send cut off
    /// when the app was closed.
    ///
    /// # Errors
    ///
    /// As for [`Engine::prepare_send_shared`]; [`BeamError::Internal`] if `transfer_id` isn't
    /// one.
    pub async fn resume_send_shared(
        &self,
        files: Vec<SharedFile>,
        transfer_id: String,
    ) -> Result<Arc<OutgoingTransfer>, BeamError> {
        let id = parse_transfer_id(&transfer_id)?;
        let opened: Vec<OpenedFile> = duplicate(&files)?;
        self.prepare(move || prepare_opened_with_id(opened, id))
            .await
    }
}

impl Engine {
    async fn prepare<F>(&self, work: F) -> Result<Arc<OutgoingTransfer>, BeamError>
    where
        F: FnOnce() -> beam_core::Result<PreparedTransfer> + Send + 'static,
    {
        let handle = self.handle()?;
        let identity = self.identity.clone();
        let task = handle.clone();
        run(&handle, async move {
            let prepared = blocking(work).await?;
            Ok(OutgoingTransfer::new(task, identity, Sender::new(prepared)))
        })
        .await
    }
}

fn to_paths(paths: Vec<String>) -> Vec<PathBuf> {
    paths.into_iter().map(PathBuf::from).collect()
}
