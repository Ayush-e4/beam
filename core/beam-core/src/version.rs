//! Protocol versioning.
//!
//! A major version mismatch means the peers cannot talk; a minor mismatch means both sides fall
//! back to the common subset of capabilities.

use std::fmt;

use crate::{BeamError, Result};

/// A protocol version exchanged in the `HELLO` frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ProtocolVersion {
    /// Incompatible changes bump the major version.
    pub major: u16,
    /// Backwards-compatible additions bump the minor version.
    pub minor: u16,
}

impl ProtocolVersion {
    /// The protocol version implemented by this build.
    pub const CURRENT: Self = Self { major: 1, minor: 0 };

    /// Checks that a peer's version can interoperate with ours.
    ///
    /// # Errors
    ///
    /// Returns [`BeamError::IncompatibleVersion`] when the major versions differ.
    pub fn ensure_compatible(self, peer: Self) -> Result<Self> {
        if self.major == peer.major {
            Ok(Self {
                major: self.major,
                minor: self.minor.min(peer.minor),
            })
        } else {
            Err(BeamError::IncompatibleVersion { local: self, peer })
        }
    }
}

impl fmt::Display for ProtocolVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}", self.major, self.minor)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const fn v(major: u16, minor: u16) -> ProtocolVersion {
        ProtocolVersion { major, minor }
    }

    #[test]
    fn same_major_negotiates_lowest_minor() {
        assert_eq!(v(1, 3).ensure_compatible(v(1, 1)).ok(), Some(v(1, 1)));
        assert_eq!(v(1, 0).ensure_compatible(v(1, 4)).ok(), Some(v(1, 0)));
    }

    #[test]
    fn different_major_is_rejected() {
        let result = v(1, 0).ensure_compatible(v(2, 0));
        assert!(matches!(result, Err(BeamError::IncompatibleVersion { .. })));
    }

    #[test]
    fn displays_as_major_dot_minor() {
        assert_eq!(ProtocolVersion::CURRENT.to_string(), "1.0");
    }
}
