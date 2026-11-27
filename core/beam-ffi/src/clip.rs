//! Clipboard items as the app sees them, and the text checks the app uses before sending.

use beam_core::{ClipContent, ClipItem, ClipLabel};

/// What a clipboard item holds.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum ClipBody {
    /// Text, including links and codes.
    Text {
        /// The text.
        text: String,
    },
    /// An image.
    Image {
        /// Its type, such as `image/png`.
        mime: String,
        /// The encoded image.
        bytes: Vec<u8>,
    },
}

/// What a clipboard item looks like, for how to show it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum ClipKind {
    /// Any other text.
    Text,
    /// A single web link.
    Link,
    /// A one-time code.
    Code,
    /// An image.
    Image,
}

/// An item the other device copied.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ReceivedClip {
    /// What it holds.
    pub body: ClipBody,
    /// What it looks like.
    pub kind: ClipKind,
    /// Keep it out of previews and history (on Android 13+, mark the clip sensitive): the
    /// sender said so, or it is a one-time code.
    pub sensitive: bool,
    /// The one-time code in it, ready to paste, if it holds one.
    pub code: Option<String>,
}

impl From<ClipItem> for ReceivedClip {
    fn from(item: ClipItem) -> Self {
        let kind = item.label().into();
        let sensitive = item.is_sensitive();
        let (body, code) = match item.content() {
            ClipContent::Text(text) => (
                ClipBody::Text { text: text.clone() },
                beam_core::find_code(text),
            ),
            ClipContent::Image { mime, bytes } => (
                ClipBody::Image {
                    mime: mime.clone(),
                    bytes: bytes.clone(),
                },
                None,
            ),
        };
        Self {
            body,
            kind,
            sensitive,
            code,
        }
    }
}

impl From<ClipLabel> for ClipKind {
    fn from(label: ClipLabel) -> Self {
        match label {
            ClipLabel::Text => Self::Text,
            ClipLabel::Link => Self::Link,
            ClipLabel::Code => Self::Code,
            ClipLabel::Image => Self::Image,
        }
    }
}

/// What copied `text` looks like: a link, a one-time code or other text.
#[uniffi::export]
#[must_use]
pub fn clip_kind(text: &str) -> ClipKind {
    beam_core::label_text(text).into()
}

/// The one-time code in `text` (a bare code, or a short message such as "Your OTP is
/// 482913"), ready to paste, or nothing.
#[uniffi::export]
#[must_use]
pub fn find_code(text: &str) -> Option<String> {
    beam_core::find_code(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn received_items_carry_their_kind_and_code() {
        let otp = ReceivedClip::from(ClipItem::text("Your OTP is 482913").unwrap());
        assert_eq!(otp.kind, ClipKind::Text, "a message, not a bare code");
        assert_eq!(otp.code.as_deref(), Some("482913"));
        assert!(!otp.sensitive);

        let bare = ReceivedClip::from(ClipItem::text("482913").unwrap());
        assert_eq!((bare.kind, bare.sensitive), (ClipKind::Code, true));

        let image = ReceivedClip::from(ClipItem::image("image/png", vec![1, 2]).unwrap());
        assert_eq!(
            image.body,
            ClipBody::Image {
                mime: "image/png".into(),
                bytes: vec![1, 2]
            }
        );
        assert_eq!((image.kind, image.code), (ClipKind::Image, None));
    }

    #[test]
    fn text_checks_match_the_engine() {
        assert_eq!(clip_kind("https://beam.app"), ClipKind::Link);
        assert_eq!(clip_kind("G-123456"), ClipKind::Code);
        assert_eq!(find_code("hello"), None);
    }
}
