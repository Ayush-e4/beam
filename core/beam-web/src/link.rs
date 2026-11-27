//! A live clipboard link with the phone, run in the background of the page or worker.

use std::cell::RefCell;
use std::rc::Rc;

use beam_core::{
    BeamError, ClipContent, ClipItem, ClipLabel, EchoGuard, LinkEnd, LinkTiming, run_link,
};
use tokio::sync::mpsc;
use wasm_bindgen::prelude::*;

use crate::BeamConnection;
use crate::errors::beam_error;

/// Items waiting to be sent or handed over, per direction.
const QUEUED_ITEMS: usize = 8;

type Ended = Rc<RefCell<Option<Result<LinkEnd, JsValue>>>>;

/// An item the phone copied.
#[wasm_bindgen(getter_with_clone)]
pub struct Clip {
    /// `"text"`, `"link"`, `"code"` or `"image"`.
    pub kind: String,
    /// The text, for every kind but images.
    pub text: Option<String>,
    /// The image's type, such as `image/png`.
    pub mime: Option<String>,
    /// The encoded image.
    pub bytes: Option<Vec<u8>>,
    /// Keep it out of previews and history: the phone said so, or it is a one-time code.
    pub sensitive: bool,
    /// The one-time code in it, ready to paste, if it holds one.
    pub code: Option<String>,
}

impl From<ClipItem> for Clip {
    fn from(item: ClipItem) -> Self {
        let kind = kind_name(item.label()).to_owned();
        let sensitive = item.is_sensitive();
        match item.content() {
            ClipContent::Text(text) => Self {
                kind,
                code: beam_core::find_code(text),
                text: Some(text.clone()),
                mime: None,
                bytes: None,
                sensitive,
            },
            ClipContent::Image { mime, bytes } => Self {
                kind,
                text: None,
                mime: Some(mime.clone()),
                bytes: Some(bytes.clone()),
                sensitive,
                code: None,
            },
        }
    }
}

/// Clipboard items to and from the phone while both stay connected. Ends when either side
/// stops, the connection drops, or the phone goes silent for 45 s.
#[wasm_bindgen]
pub struct BeamLink {
    outgoing: RefCell<Option<mpsc::Sender<ClipItem>>>,
    incoming: Rc<tokio::sync::Mutex<mpsc::Receiver<ClipItem>>>,
    ended: Ended,
    echo: RefCell<EchoGuard>,
}

#[wasm_bindgen]
impl BeamConnection {
    /// Turns this connection into a clipboard link, which runs in the background. The
    /// connection can't be used for anything else afterwards.
    #[must_use]
    pub fn link(self) -> BeamLink {
        let mut conn = self.connection;
        let (outgoing, to_send) = mpsc::channel(QUEUED_ITEMS);
        let (arrived, incoming) = mpsc::channel(QUEUED_ITEMS);
        let ended = Ended::default();
        let record = ended.clone();
        wasm_bindgen_futures::spawn_local(async move {
            // Keep a sender until the end is recorded, so `nextClip` sees why it ended.
            let keep_open = arrived.clone();
            let end = run_link(&mut conn, to_send, arrived, LinkTiming::default()).await;
            *record.borrow_mut() = Some(end.map_err(beam_error));
            drop(keep_open);
        });
        BeamLink {
            outgoing: RefCell::new(Some(outgoing)),
            incoming: Rc::new(tokio::sync::Mutex::new(incoming)),
            ended,
            echo: RefCell::default(),
        }
    }
}

#[wasm_bindgen]
impl BeamLink {
    /// Sends copied text; mark passwords and codes `sensitive`. Resolves `false` if it is what
    /// the phone just sent or what was sent last.
    ///
    /// # Errors
    ///
    /// A protocol error if the text is empty or over 1 MiB, the link's error once it ended.
    #[wasm_bindgen(js_name = sendText)]
    pub async fn send_text(&self, text: String, sensitive: bool) -> Result<bool, JsValue> {
        let item = ClipItem::text(text).map_err(beam_error)?;
        self.send(item.with_sensitive(sensitive)).await
    }

    /// Sends a copied image (`image/png`, `image/jpeg`, …, at most 8 MiB), like `sendText`.
    ///
    /// # Errors
    ///
    /// A protocol error for an unsupported type or size, the link's error once it ended.
    #[wasm_bindgen(js_name = sendImage)]
    pub async fn send_image(
        &self,
        mime: String,
        bytes: Vec<u8>,
        sensitive: bool,
    ) -> Result<bool, JsValue> {
        let item = ClipItem::image(&mime, bytes).map_err(beam_error)?;
        self.send(item.with_sensitive(sensitive)).await
    }

    /// The next item the phone copied, or `undefined` once the link ended cleanly. Writing it
    /// to the clipboard won't send it back.
    ///
    /// # Errors
    ///
    /// Why the link failed: a network error if the connection dropped or the phone went
    /// silent, a protocol error if it sent something invalid.
    #[wasm_bindgen(js_name = nextClip)]
    pub async fn next_clip(&self) -> Result<Option<Clip>, JsValue> {
        let item = self.incoming.lock().await.recv().await;
        let Some(item) = item else {
            return match self.ended.borrow_mut().take() {
                Some(Err(error)) => Err(error),
                Some(Ok(_)) | None => Ok(None),
            };
        };
        self.echo.borrow_mut().received(&item);
        Ok(Some(item.into()))
    }

    /// Ends the link cleanly: the phone sees it close. Items already queued go first.
    pub fn stop(&self) {
        self.outgoing.borrow_mut().take();
    }
}

impl BeamLink {
    async fn send(&self, item: ClipItem) -> Result<bool, JsValue> {
        if !self.echo.borrow_mut().should_send(&item) {
            return Ok(false);
        }
        let outgoing = self.outgoing.borrow().clone().ok_or_else(stopped)?;
        if outgoing.send(item).await.is_ok() {
            return Ok(true);
        }
        Err(match self.ended.borrow().as_ref() {
            Some(Err(error)) => error.clone(),
            _ => stopped(),
        })
    }
}

/// What copied `text` looks like: `"link"`, `"code"` or `"text"`.
#[wasm_bindgen(js_name = clipKind)]
#[must_use]
pub fn clip_kind(text: &str) -> String {
    kind_name(beam_core::label_text(text)).to_owned()
}

/// The one-time code in `text`, ready to paste, or `undefined`.
#[wasm_bindgen(js_name = findCode)]
#[must_use]
pub fn find_code(text: &str) -> Option<String> {
    beam_core::find_code(text)
}

const fn kind_name(label: ClipLabel) -> &'static str {
    match label {
        ClipLabel::Text => "text",
        ClipLabel::Link => "link",
        ClipLabel::Code => "code",
        ClipLabel::Image => "image",
    }
}

fn stopped() -> JsValue {
    beam_error(BeamError::Cancelled)
}
