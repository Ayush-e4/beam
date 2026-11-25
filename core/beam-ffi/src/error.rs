//! The error type every exported call returns.
//!
//! Platform code switches on the category to pick a message and an action (retry, free space,
//! grant a permission...). `detail` is the engine's description, meant for logs and bug reports
//! rather than for showing to people.

use beam_core::ErrorCategory;

/// An engine failure, by category. Kotlin sees it as `BeamException`.
#[derive(Debug, PartialEq, Eq, thiserror::Error, uniffi::Error)]
pub enum BeamError {
    /// The link to the peer failed: unreachable, dropped or timed out. Usually worth retrying.
    #[error("network: {detail}")]
    Network {
        /// What went wrong, for logs.
        detail: String,
    },
    /// Reading or writing local files failed, or the picked files cannot be sent as they are.
    #[error("storage: {detail}")]
    Storage {
        /// What went wrong, for logs.
        detail: String,
    },
    /// A required OS permission is missing.
    #[error("permission: {detail}")]
    Permission {
        /// What went wrong, for logs.
        detail: String,
    },
    /// The peer broke the protocol or runs an incompatible version.
    #[error("protocol: {detail}")]
    Protocol {
        /// What went wrong, for logs.
        detail: String,
    },
    /// Pairing, encryption or an integrity check failed; the peer may not be who it claims.
    #[error("crypto: {detail}")]
    Crypto {
        /// What went wrong, for logs.
        detail: String,
    },
    /// The user or the peer cancelled.
    #[error("cancelled: {detail}")]
    Cancelled {
        /// What went wrong, for logs.
        detail: String,
    },
}

impl From<beam_core::BeamError> for BeamError {
    fn from(error: beam_core::BeamError) -> Self {
        let detail = error.to_string();
        match error.category() {
            ErrorCategory::Network => Self::Network { detail },
            ErrorCategory::Storage => Self::Storage { detail },
            ErrorCategory::Permission => Self::Permission { detail },
            ErrorCategory::Protocol => Self::Protocol { detail },
            ErrorCategory::Crypto => Self::Crypto { detail },
            ErrorCategory::Cancelled => Self::Cancelled { detail },
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io;

    use beam_core::BeamError as Core;

    use super::*;

    fn convert(error: Core) -> BeamError {
        error.into()
    }

    #[test]
    fn each_category_maps_to_its_own_variant() {
        let io = |kind| io::Error::new(kind, "boom");
        assert!(matches!(
            convert(Core::network(io(io::ErrorKind::ConnectionReset))),
            BeamError::Network { .. }
        ));
        assert!(matches!(
            convert(Core::storage(io(io::ErrorKind::StorageFull))),
            BeamError::Storage { .. }
        ));
        assert!(matches!(
            convert(Core::Io {
                category: ErrorCategory::Permission,
                source: io(io::ErrorKind::PermissionDenied),
            }),
            BeamError::Permission { .. }
        ));
        assert!(matches!(
            convert(Core::Malformed("bad frame".into())),
            BeamError::Protocol { .. }
        ));
        assert!(matches!(
            convert(Core::RecordRejected),
            BeamError::Crypto { .. }
        ));
        assert!(matches!(
            convert(Core::Cancelled),
            BeamError::Cancelled { .. }
        ));
    }

    #[test]
    fn the_detail_keeps_the_engine_message() {
        let error = convert(Core::Selection("nothing to send".into()));
        assert_eq!(
            error,
            BeamError::Storage {
                detail: "cannot send this selection: nothing to send".into()
            }
        );
    }
}
