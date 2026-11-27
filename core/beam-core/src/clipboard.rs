//! `CLIPBOARD`: one copied item, sent whole in a single frame on the control stream.
//!
//! ```text
//! CLIPBOARD : kind u8 (1 text, 2 image) | flags u8 (bit 0: sensitive)
//!             | mime_len u8 | mime (ASCII) | body (the rest: UTF-8 text or image bytes)
//! ```
//!
//! Text is `text/plain` and at most [`MAX_TEXT_LEN`] bytes; images are one of
//! [`IMAGE_TYPES`] and at most [`MAX_IMAGE_LEN`] bytes (bigger ones go as a file transfer).
//! "Sensitive" asks the receiver to hide the item from previews and history, as Android does
//! for passwords and one-time codes.
//!
//! [`EchoGuard`] stops the item a device just received from going straight back: writing it
//! to the local clipboard looks like a new copy to the app that watches for one.

use crate::clipboard_label::{ClipLabel, label_text};
use crate::frame::{Frame, FrameType};
use crate::wire::{Reader, Writer};
use crate::{BeamError, Result};

/// Stream for clipboard frames (the control stream).
pub const CLIPBOARD_STREAM: u32 = 0;
/// Longest text item, in bytes (1 MiB).
pub const MAX_TEXT_LEN: usize = 1024 * 1024;
/// Largest image item, in bytes (8 MiB).
pub const MAX_IMAGE_LEN: usize = 8 * 1024 * 1024;
/// Image types a clipboard item may carry.
pub const IMAGE_TYPES: [&str; 8] = [
    "image/png",
    "image/jpeg",
    "image/gif",
    "image/webp",
    "image/avif",
    "image/heic",
    "image/heif",
    "image/bmp",
];

const TEXT_MIME: &str = "text/plain";
const KIND_TEXT: u8 = 1;
const KIND_IMAGE: u8 = 2;
const FLAG_SENSITIVE: u8 = 1;

/// What was copied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClipContent {
    /// Plain text, including links and one-time codes.
    Text(String),
    /// An image.
    Image {
        /// One of [`IMAGE_TYPES`].
        mime: String,
        /// The encoded image.
        bytes: Vec<u8>,
    },
}

/// One clipboard item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClipItem {
    content: ClipContent,
    sensitive: bool,
}

impl ClipItem {
    /// A text item.
    ///
    /// # Errors
    ///
    /// [`BeamError::Malformed`] if `text` is empty or longer than [`MAX_TEXT_LEN`].
    pub fn text(text: impl Into<String>) -> Result<Self> {
        let text = text.into();
        check_text(&text)?;
        Ok(Self {
            content: ClipContent::Text(text),
            sensitive: false,
        })
    }

    /// An image item.
    ///
    /// # Errors
    ///
    /// [`BeamError::Malformed`] if `mime` isn't one of [`IMAGE_TYPES`], or `bytes` is empty or
    /// larger than [`MAX_IMAGE_LEN`].
    pub fn image(mime: &str, bytes: Vec<u8>) -> Result<Self> {
        check_image(mime, &bytes)?;
        Ok(Self {
            content: ClipContent::Image {
                mime: mime.to_owned(),
                bytes,
            },
            sensitive: false,
        })
    }

    /// The same item, marked sensitive or not.
    #[must_use]
    pub const fn with_sensitive(mut self, sensitive: bool) -> Self {
        self.sensitive = sensitive;
        self
    }

    /// What was copied.
    #[must_use]
    pub const fn content(&self) -> &ClipContent {
        &self.content
    }

    /// What kind of item it looks like, for the UI.
    #[must_use]
    pub fn label(&self) -> ClipLabel {
        match &self.content {
            ClipContent::Text(text) => label_text(text),
            ClipContent::Image { .. } => ClipLabel::Image,
        }
    }

    /// Whether to keep it out of previews and history: the sender said so, or it looks like a
    /// one-time code.
    #[must_use]
    pub fn is_sensitive(&self) -> bool {
        self.sensitive || self.label() == ClipLabel::Code
    }

    /// A hash of the content (not the flags), to recognise the same item again.
    #[must_use]
    pub fn fingerprint(&self) -> [u8; 32] {
        let mut hasher = blake3::Hasher::new();
        let (kind, mime, body) = self.parts();
        hasher.update(&[kind]).update(mime.as_bytes()).update(&[0]);
        hasher.update(body);
        *hasher.finalize().as_bytes()
    }

    /// The `CLIPBOARD` payload.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let (kind, mime, body) = self.parts();
        let flags = if self.sensitive { FLAG_SENSITIVE } else { 0 };
        let mime_len = u8::try_from(mime.len()).unwrap_or(0); // every allowed type is short
        Writer::new()
            .u8(kind)
            .u8(flags)
            .u8(mime_len)
            .bytes(mime.as_bytes())
            .bytes(body)
            .finish()
    }

    /// The item as a frame, ready to send.
    ///
    /// # Errors
    ///
    /// [`BeamError::Malformed`] only if the payload were over the frame limit, which the item
    /// limits rule out.
    pub fn to_frame(&self) -> Result<Frame> {
        Frame::new(FrameType::Clipboard, CLIPBOARD_STREAM, self.encode())
    }

    /// Parses a `CLIPBOARD` payload.
    ///
    /// # Errors
    ///
    /// [`BeamError::Malformed`] for an unknown kind or flag, a truncated payload, text that
    /// isn't UTF-8, a type that doesn't match the kind, or content over the limits.
    pub fn decode(payload: &[u8]) -> Result<Self> {
        let mut reader = Reader::new(payload);
        let kind = reader.u8()?;
        let flags = reader.u8()?;
        if flags & !FLAG_SENSITIVE != 0 {
            return Err(BeamError::Malformed("unknown clipboard flags".into()));
        }
        let mime_len = usize::from(reader.u8()?);
        let mime = reader.utf8(mime_len)?;
        let body = reader.rest();
        let item = match kind {
            KIND_TEXT if mime == TEXT_MIME => {
                let text = std::str::from_utf8(body)
                    .map_err(|_| BeamError::Malformed("clipboard text is not UTF-8".into()))?;
                Self::text(text)?
            }
            KIND_IMAGE => Self::image(mime, body.to_vec())?,
            KIND_TEXT => {
                return Err(BeamError::Malformed(format!(
                    "clipboard text has type {mime:?}"
                )));
            }
            other => {
                return Err(BeamError::Malformed(format!(
                    "unknown clipboard kind {other}"
                )));
            }
        };
        Ok(item.with_sensitive(flags & FLAG_SENSITIVE != 0))
    }

    fn parts(&self) -> (u8, &str, &[u8]) {
        match &self.content {
            ClipContent::Text(text) => (KIND_TEXT, TEXT_MIME, text.as_bytes()),
            ClipContent::Image { mime, bytes } => (KIND_IMAGE, mime, bytes),
        }
    }
}

/// Remembers the last item that crossed, either way, so it isn't sent again.
#[derive(Debug, Default)]
pub struct EchoGuard {
    last: Option<[u8; 32]>,
}

impl EchoGuard {
    /// A guard that has seen nothing yet.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Notes an item from the peer that is being put on the local clipboard.
    pub fn received(&mut self, item: &ClipItem) {
        self.last = Some(item.fingerprint());
    }

    /// Whether a local copy should go to the peer: not if it is what just arrived or what was
    /// just sent. Returning `true` notes it as sent.
    pub fn should_send(&mut self, item: &ClipItem) -> bool {
        let fingerprint = item.fingerprint();
        if self.last == Some(fingerprint) {
            return false;
        }
        self.last = Some(fingerprint);
        true
    }
}

fn check_text(text: &str) -> Result<()> {
    if text.is_empty() {
        return Err(BeamError::Malformed("clipboard text is empty".into()));
    }
    if text.len() > MAX_TEXT_LEN {
        return Err(BeamError::Malformed(format!(
            "clipboard text is over {MAX_TEXT_LEN} bytes"
        )));
    }
    Ok(())
}

fn check_image(mime: &str, bytes: &[u8]) -> Result<()> {
    if !IMAGE_TYPES.contains(&mime) {
        return Err(BeamError::Malformed(format!(
            "clipboard image type {mime:?} is not supported"
        )));
    }
    if bytes.is_empty() {
        return Err(BeamError::Malformed("clipboard image is empty".into()));
    }
    if bytes.len() > MAX_IMAGE_LEN {
        return Err(BeamError::Malformed(format!(
            "clipboard image is over {MAX_IMAGE_LEN} bytes"
        )));
    }
    Ok(())
}

#[cfg(test)]
#[path = "clipboard_tests.rs"]
mod tests;
