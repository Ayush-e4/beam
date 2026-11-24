//! Typed errors shared by every part of the engine.
//!
//! Each error carries a stable [`ErrorCategory`] so platform layers can map it to a user-facing
//! message and a suggested action without parsing strings.

use thiserror::Error;

/// Result type used throughout `beam-core`.
pub type Result<T> = std::result::Result<T, BeamError>;

/// Broad error category, stable across versions and exposed to platform layers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ErrorCategory {
    /// Connectivity problems: unreachable peer, dropped link, timeouts.
    Network,
    /// Local storage problems: disk full, file missing, write failure.
    Storage,
    /// A required OS permission is missing.
    Permission,
    /// The peer sent something that violates the protocol.
    Protocol,
    /// Handshake, key or integrity verification failure.
    Crypto,
    /// The user or peer cancelled the operation.
    Cancelled,
}

/// Errors produced by the Beam engine.
#[derive(Debug, Error)]
pub enum BeamError {
    /// The peer speaks an incompatible protocol major version.
    #[error("incompatible protocol version: local {local}, peer {peer}")]
    IncompatibleVersion {
        /// Our protocol version.
        local: crate::ProtocolVersion,
        /// The peer's protocol version.
        peer: crate::ProtocolVersion,
    },

    /// A received message could not be parsed or was invalid.
    #[error("malformed message: {0}")]
    Malformed(String),

    /// A path received from a peer is unsafe (traversal, absolute path, invalid name).
    #[error("unsafe path {path:?}: {reason}")]
    UnsafePath {
        /// The path as received.
        path: String,
        /// Why it was rejected.
        reason: &'static str,
    },

    /// An I/O error from the network or the file system.
    #[error("{category:?} I/O error: {source}")]
    Io {
        /// Whether the failure came from the network or storage.
        category: ErrorCategory,
        /// The underlying I/O error.
        #[source]
        source: std::io::Error,
    },

    /// Received data does not match its hash.
    #[error("integrity check failed for chunk {chunk}")]
    IntegrityMismatch {
        /// Index of the chunk that failed verification.
        chunk: u64,
    },

    /// The operation was cancelled.
    #[error("operation cancelled")]
    Cancelled,
}

impl BeamError {
    /// Returns the stable category of this error.
    #[must_use]
    pub fn category(&self) -> ErrorCategory {
        match self {
            Self::IncompatibleVersion { .. } | Self::Malformed(_) | Self::UnsafePath { .. } => {
                ErrorCategory::Protocol
            }
            Self::Io { category, .. } => *category,
            Self::IntegrityMismatch { .. } => ErrorCategory::Crypto,
            Self::Cancelled => ErrorCategory::Cancelled,
        }
    }

    /// Wraps a storage-related I/O error.
    #[must_use]
    pub fn storage(source: std::io::Error) -> Self {
        Self::Io {
            category: ErrorCategory::Storage,
            source,
        }
    }

    /// Wraps a network-related I/O error.
    #[must_use]
    pub fn network(source: std::io::Error) -> Self {
        Self::Io {
            category: ErrorCategory::Network,
            source,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io;

    #[test]
    fn io_errors_keep_their_category() {
        let err = BeamError::storage(io::Error::new(io::ErrorKind::StorageFull, "disk full"));
        assert_eq!(err.category(), ErrorCategory::Storage);

        let err = BeamError::network(io::Error::new(io::ErrorKind::TimedOut, "timeout"));
        assert_eq!(err.category(), ErrorCategory::Network);
    }

    #[test]
    fn protocol_errors_are_categorized() {
        let err = BeamError::Malformed("bad frame".into());
        assert_eq!(err.category(), ErrorCategory::Protocol);
        assert_eq!(err.to_string(), "malformed message: bad frame");
    }

    #[test]
    fn cancelled_is_its_own_category() {
        assert_eq!(BeamError::Cancelled.category(), ErrorCategory::Cancelled);
    }
}
