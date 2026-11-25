//! Plain data that crosses the boundary: ciphers, offers and outcomes.

use beam_core::messages::Reason;
use beam_core::{Cipher, Manifest, PublicKey, SendOutcome, TransferId};

use crate::BeamError;

/// The session cipher; both devices must use the same one (chosen when pairing).
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum SessionCipher {
    /// AES-256-GCM: fastest where the CPU accelerates AES.
    AesGcm,
    /// ChaCha20-Poly1305: for CPUs without AES acceleration.
    ChaChaPoly,
}

impl From<SessionCipher> for Cipher {
    fn from(cipher: SessionCipher) -> Self {
        match cipher {
            SessionCipher::AesGcm => Self::AesGcm,
            SessionCipher::ChaChaPoly => Self::ChaChaPoly,
        }
    }
}

impl From<Cipher> for SessionCipher {
    fn from(cipher: Cipher) -> Self {
        match cipher {
            Cipher::AesGcm => Self::AesGcm,
            Cipher::ChaChaPoly => Self::ChaChaPoly,
        }
    }
}

/// Why a receiver declined an offer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum DeclineReason {
    /// The person said no.
    UserDeclined,
    /// Not enough free space.
    NoSpace,
    /// Busy with another transfer.
    Busy,
    /// Something went wrong on the receiver.
    Failed,
}

impl From<DeclineReason> for Reason {
    fn from(reason: DeclineReason) -> Self {
        match reason {
            DeclineReason::UserDeclined => Self::UserDeclined,
            DeclineReason::NoSpace => Self::NoSpace,
            DeclineReason::Busy => Self::Busy,
            DeclineReason::Failed => Self::Failed,
        }
    }
}

impl From<Reason> for DeclineReason {
    fn from(reason: Reason) -> Self {
        match reason {
            Reason::UserDeclined => Self::UserDeclined,
            Reason::NoSpace => Self::NoSpace,
            Reason::Busy => Self::Busy,
            Reason::Failed => Self::Failed,
        }
    }
}

/// How a send ended when it did not fail.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum SendResult {
    /// Every accepted file was verified and saved by the receiver.
    Delivered,
    /// The receiver declined.
    Declined {
        /// Why.
        reason: DeclineReason,
    },
}

impl From<SendOutcome> for SendResult {
    fn from(outcome: SendOutcome) -> Self {
        match outcome {
            SendOutcome::Delivered => Self::Delivered,
            SendOutcome::Rejected(reason) => Self::Declined {
                reason: reason.into(),
            },
        }
    }
}

/// One file in a transfer.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct OfferedItem {
    /// ID to pass when accepting only some files.
    pub item_id: u32,
    /// Relative path, `/`-separated, as it will be saved.
    pub path: String,
    /// Size in bytes.
    pub size: u64,
    /// MIME type guessed from the extension.
    pub mime: String,
}

/// What a transfer contains.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct TransferSummary {
    /// The transfer's ID, 32 hex characters; the same across reconnects.
    pub transfer_id: String,
    /// Every file, in manifest order.
    pub items: Vec<OfferedItem>,
    /// Sum of all file sizes.
    pub total_bytes: u64,
}

impl From<&Manifest> for TransferSummary {
    fn from(manifest: &Manifest) -> Self {
        Self {
            transfer_id: hex(&manifest.transfer_id()),
            items: manifest
                .items()
                .iter()
                .map(|item| OfferedItem {
                    item_id: item.item_id,
                    path: item.path.to_string(),
                    size: item.size,
                    mime: item.mime.clone(),
                })
                .collect(),
            total_bytes: manifest.total_size(),
        }
    }
}

fn hex(id: &TransferId) -> String {
    use std::fmt::Write as _;
    id.0.iter().fold(String::new(), |mut out, byte| {
        let _ = write!(out, "{byte:02x}"); // writing to a String cannot fail
        out
    })
}

/// Parses a peer's 32-byte public key.
pub(crate) fn peer_key(bytes: &[u8]) -> Result<PublicKey, BeamError> {
    let array: [u8; 32] = bytes.try_into().map_err(|_| BeamError::Crypto {
        detail: format!("a device public key is 32 bytes, not {}", bytes.len()),
    })?;
    Ok(PublicKey::from_bytes(array))
}

/// What [`crate::Engine::remove_stale_transfers`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Record)]
pub struct UnfinishedCleanup {
    /// Unfinished transfers removed.
    pub removed: u32,
    /// Stale ones that couldn't be removed (for example, files an earlier install left).
    pub failed: u32,
}

impl From<beam_core::staging::Cleanup> for UnfinishedCleanup {
    fn from(cleanup: beam_core::staging::Cleanup) -> Self {
        Self {
            removed: cleanup.removed,
            failed: cleanup.failed,
        }
    }
}
