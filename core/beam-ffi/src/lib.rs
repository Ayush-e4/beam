//! The boundary between `beam-core` and the platform apps.
//!
//! Everything exported here becomes Kotlin through UniFFI (package `app.beam.core`). Keep this
//! layer thin: it converts types and errors, and the engine does the work. Every fallible call
//! returns a typed [`BeamError`], which Kotlin sees as `BeamException` with one subclass per
//! category.

mod engine;
mod error;
mod identity;
mod incoming;
mod outgoing;
mod progress;
mod runtime;
mod types;

pub use engine::Engine;
pub use error::BeamError;
pub use identity::DeviceIdentity;
pub use incoming::{IncomingOffer, Listener};
pub use outgoing::OutgoingTransfer;
pub use progress::ProgressListener;
pub use types::{DeclineReason, OfferedItem, SendResult, SessionCipher, TransferSummary};

use beam_core::ProtocolVersion;

uniffi::setup_scaffolding!();

/// Which engine and protocol this build implements.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct VersionInfo {
    /// The engine's crate version, for diagnostics and bug reports.
    pub engine: String,
    /// Protocol major version; peers with a different major cannot talk.
    pub protocol_major: u16,
    /// Protocol minor version; peers fall back to the lower minor.
    pub protocol_minor: u16,
}

/// The engine and protocol versions of this build.
#[uniffi::export]
#[must_use]
pub fn version_info() -> VersionInfo {
    let protocol = ProtocolVersion::CURRENT;
    VersionInfo {
        engine: env!("CARGO_PKG_VERSION").to_owned(),
        protocol_major: protocol.major,
        protocol_minor: protocol.minor,
    }
}

/// Checks a peer's advertised protocol version (for example from discovery) before connecting,
/// and returns the minor version both sides will use.
///
/// # Errors
///
/// [`BeamError::Protocol`] when the major versions differ.
#[uniffi::export]
pub fn negotiate_protocol(peer_major: u16, peer_minor: u16) -> Result<u16, BeamError> {
    let peer = ProtocolVersion {
        major: peer_major,
        minor: peer_minor,
    };
    Ok(ProtocolVersion::CURRENT.ensure_compatible(peer)?.minor)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_info_matches_the_engine() {
        let info = version_info();
        assert_eq!(info.engine, env!("CARGO_PKG_VERSION"));
        assert_eq!(info.protocol_major, ProtocolVersion::CURRENT.major);
        assert_eq!(info.protocol_minor, ProtocolVersion::CURRENT.minor);
    }

    #[test]
    fn a_newer_minor_negotiates_down_to_ours() {
        let ours = ProtocolVersion::CURRENT;
        assert_eq!(
            negotiate_protocol(ours.major, ours.minor + 1).ok(),
            Some(ours.minor)
        );
    }

    #[test]
    fn a_different_major_is_a_protocol_error() {
        let ours = ProtocolVersion::CURRENT;
        let error = negotiate_protocol(ours.major + 1, 0).err();
        assert!(
            matches!(error, Some(BeamError::Protocol { .. })),
            "{error:?}"
        );
    }
}
