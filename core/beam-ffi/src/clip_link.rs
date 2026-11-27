//! A live clipboard link with another device, run on the engine's runtime.

use std::sync::{Arc, Mutex, PoisonError};

use beam_core::{ClipItem, Connection, EchoGuard, LinkEnd, LinkTiming, run_link};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::runtime::Handle;
use tokio::sync::mpsc;
use tokio::task::AbortHandle;

use crate::BeamError;
use crate::clip::ReceivedClip;
use crate::runtime::run;

/// Items waiting to be sent or handed over, per direction.
const QUEUED_ITEMS: usize = 8;

type Ended = Arc<Mutex<Option<Result<LinkEnd, BeamError>>>>;

/// Whatever must outlive the link, such as the server its connection came through.
pub(crate) type KeepAlive = Arc<dyn std::any::Any + Send + Sync>;

/// Clipboard items to and from one paired device, while both stay connected. Send what the
/// person copies; [`Self::next_clip`] gives what the other device copied. Ends when either
/// side stops, the connection drops, or the other device goes silent for 45 s.
#[derive(uniffi::Object)]
pub struct ClipLink {
    handle: Handle,
    peer_key: Vec<u8>,
    outgoing: Mutex<Option<mpsc::Sender<ClipItem>>>,
    incoming: Arc<tokio::sync::Mutex<mpsc::Receiver<ClipItem>>>,
    ended: Ended,
    echo: Mutex<EchoGuard>,
    task: AbortHandle,
    _keep_alive: Option<KeepAlive>,
}

impl std::fmt::Debug for ClipLink {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ClipLink").finish_non_exhaustive()
    }
}

impl ClipLink {
    /// Starts the link over `conn` on `handle`, holding on to `keep_alive` until it is
    /// released.
    pub(crate) fn start<S>(
        handle: Handle,
        mut conn: Connection<S>,
        keep_alive: Option<KeepAlive>,
    ) -> Arc<Self>
    where
        S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    {
        let peer_key = conn.peer().as_bytes().to_vec();
        let (outgoing, to_send) = mpsc::channel(QUEUED_ITEMS);
        let (arrived, incoming) = mpsc::channel(QUEUED_ITEMS);
        let ended = Ended::default();
        let record = ended.clone();
        let task = handle
            .spawn(async move {
                // Keep a sender until the end is recorded, so `next_clip` sees why it ended.
                let keep_open = arrived.clone();
                let end = run_link(&mut conn, to_send, arrived, LinkTiming::default()).await;
                *lock(&record) = Some(end.map_err(BeamError::from));
                drop(keep_open);
            })
            .abort_handle();
        Arc::new(Self {
            handle,
            peer_key,
            outgoing: Mutex::new(Some(outgoing)),
            incoming: Arc::new(tokio::sync::Mutex::new(incoming)),
            ended,
            echo: Mutex::default(),
            task,
            _keep_alive: keep_alive,
        })
    }

    async fn send(&self, item: ClipItem) -> Result<bool, BeamError> {
        if !lock(&self.echo).should_send(&item) {
            return Ok(false);
        }
        let outgoing = lock(&self.outgoing).clone().ok_or_else(stopped)?;
        let ended = self.ended.clone();
        run(&self.handle, async move {
            match outgoing.send(item).await {
                Ok(()) => Ok(true),
                Err(_) => Err(end_error(&ended)),
            }
        })
        .await
    }
}

#[uniffi::export]
impl ClipLink {
    /// The other device's public key.
    #[must_use]
    pub fn peer_key(&self) -> Vec<u8> {
        self.peer_key.clone()
    }

    /// Sends copied text; mark passwords and codes `sensitive`. False if it is what the other
    /// device just sent (writing it here looked like a new copy) or what was sent last.
    ///
    /// # Errors
    ///
    /// [`BeamError::Protocol`] if the text is empty or over 1 MiB; the link's error, or
    /// [`BeamError::Cancelled`], once the link has ended.
    pub async fn send_text(&self, text: String, sensitive: bool) -> Result<bool, BeamError> {
        self.send(ClipItem::text(text)?.with_sensitive(sensitive))
            .await
    }

    /// Sends a copied image (`image/png`, `image/jpeg`, …, at most 8 MiB), like
    /// [`Self::send_text`].
    ///
    /// # Errors
    ///
    /// [`BeamError::Protocol`] for an unsupported type, an empty or too large image; the
    /// link's error once it has ended.
    pub async fn send_image(
        &self,
        mime: String,
        bytes: Vec<u8>,
        sensitive: bool,
    ) -> Result<bool, BeamError> {
        self.send(ClipItem::image(&mime, bytes)?.with_sensitive(sensitive))
            .await
    }

    /// The next item the other device copied, or nothing once the link ended cleanly. Put it
    /// on the clipboard; it won't be sent back.
    ///
    /// # Errors
    ///
    /// Why the link failed: [`BeamError::Network`] if the connection dropped or the device
    /// went silent, [`BeamError::Protocol`] if it sent something invalid.
    pub async fn next_clip(&self) -> Result<Option<ReceivedClip>, BeamError> {
        let incoming = self.incoming.clone();
        let ended = self.ended.clone();
        let item = run(&self.handle, async move {
            match incoming.lock().await.recv().await {
                Some(item) => Ok(Some(item)),
                None => match lock(&ended).take() {
                    Some(Err(error)) => Err(error),
                    Some(Ok(_)) | None => Ok(None),
                },
            }
        })
        .await?;
        Ok(item.map(|item| {
            lock(&self.echo).received(&item);
            item.into()
        }))
    }

    /// Ends the link cleanly: the other device sees it close. Items already queued go first.
    pub fn stop(&self) {
        lock(&self.outgoing).take();
    }
}

impl Drop for ClipLink {
    fn drop(&mut self) {
        self.task.abort();
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    // Nothing here can be left half-updated by a panic: each guard sets or takes one value.
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

fn end_error(ended: &Ended) -> BeamError {
    match lock(ended).as_ref() {
        Some(Err(error)) => error.clone(),
        _ => stopped(),
    }
}

fn stopped() -> BeamError {
    BeamError::Cancelled {
        detail: "the clipboard link ended".into(),
    }
}
