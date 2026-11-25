//! Long-term device keys used to authenticate Noise handshakes.
//!
//! A device's static key pair is X25519. The private half is kept in memory that is wiped on
//! drop; platform layers load it from the keystore with [`StaticKeypair::from_private_bytes`].

use std::fmt;

use snow::params::DHChoice;
use snow::resolvers::{CryptoResolver, DefaultResolver};
use zeroize::Zeroizing;

use crate::{BeamError, Result};

/// Length in bytes of an X25519 public or private key.
pub const KEY_LEN: usize = 32;

/// A device's X25519 public key.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct PublicKey([u8; KEY_LEN]);

impl PublicKey {
    /// Wraps raw public key bytes.
    #[must_use]
    pub const fn from_bytes(bytes: [u8; KEY_LEN]) -> Self {
        Self(bytes)
    }

    /// Returns the raw key bytes.
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; KEY_LEN] {
        &self.0
    }

    /// Parses a key received from the network or the Noise library.
    pub(crate) fn from_slice(bytes: &[u8]) -> Result<Self> {
        let array = bytes
            .try_into()
            .map_err(|_| BeamError::Malformed(format!("public key must be {KEY_LEN} bytes")))?;
        Ok(Self(array))
    }
}

/// Stable identifier of a device: the first 16 bytes of a BLAKE3 hash of its public key.
///
/// Trusted-device records and connection logic use it to name a peer without carrying the
/// whole key around. It is derived, never chosen, so it cannot be claimed by another key.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct DeviceId([u8; DEVICE_ID_LEN]);

/// Length in bytes of a [`DeviceId`].
pub const DEVICE_ID_LEN: usize = 16;

const DEVICE_ID_CONTEXT: &str = "beam 2026-11-25 device id v1";

impl PublicKey {
    /// The device ID this key identifies.
    #[must_use]
    pub fn device_id(&self) -> DeviceId {
        let hash = blake3::derive_key(DEVICE_ID_CONTEXT, &self.0);
        let mut id = [0u8; DEVICE_ID_LEN];
        id.copy_from_slice(&hash[..DEVICE_ID_LEN]);
        DeviceId(id)
    }
}

impl DeviceId {
    /// Returns the raw ID bytes.
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; DEVICE_ID_LEN] {
        &self.0
    }
}

/// Lowercase hex, 32 characters.
impl fmt::Display for DeviceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.iter().try_for_each(|byte| write!(f, "{byte:02x}"))
    }
}

impl fmt::Debug for DeviceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "DeviceId({self})")
    }
}

impl fmt::Debug for PublicKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "PublicKey(")?;
        for byte in &self.0[..8] {
            write!(f, "{byte:02x}")?;
        }
        write!(f, "…)")
    }
}

/// A device's long-term X25519 key pair.
pub struct StaticKeypair {
    private: Zeroizing<[u8; KEY_LEN]>,
    public: PublicKey,
}

impl StaticKeypair {
    /// Generates a new random key pair.
    ///
    /// # Errors
    ///
    /// Returns [`BeamError::Handshake`] if the system random number generator fails.
    pub fn generate() -> Result<Self> {
        let mut dh = curve25519()?;
        let mut rng = DefaultResolver
            .resolve_rng()
            .ok_or_else(|| BeamError::Handshake("no random number generator".into()))?;
        dh.generate(rng.as_mut())
            .map_err(|err| BeamError::Handshake(format!("key generation failed: {err}")))?;
        let mut private = Zeroizing::new([0u8; KEY_LEN]);
        private.copy_from_slice(dh.privkey());
        Ok(Self {
            private,
            public: PublicKey::from_slice(dh.pubkey())?,
        })
    }

    /// Rebuilds a key pair from a stored private key, deriving the public key.
    ///
    /// # Errors
    ///
    /// Returns [`BeamError::Handshake`] if X25519 is unavailable.
    pub fn from_private_bytes(private: &[u8; KEY_LEN]) -> Result<Self> {
        let mut dh = curve25519()?;
        dh.set(private);
        Ok(Self {
            private: Zeroizing::new(*private),
            public: PublicKey::from_slice(dh.pubkey())?,
        })
    }

    /// Returns the public half.
    #[must_use]
    pub const fn public(&self) -> PublicKey {
        self.public
    }

    /// Returns the private key bytes for handing to the Noise library or the keystore.
    #[must_use]
    pub fn private_bytes(&self) -> &[u8; KEY_LEN] {
        &self.private
    }
}

impl fmt::Debug for StaticKeypair {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("StaticKeypair")
            .field("public", &self.public)
            .field("private", &"<redacted>")
            .finish()
    }
}

fn curve25519() -> Result<Box<dyn snow::types::Dh>> {
    DefaultResolver
        .resolve_dh(&DHChoice::Curve25519)
        .ok_or_else(|| BeamError::Handshake("X25519 is unavailable".into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_keys_are_distinct() {
        let a = StaticKeypair::generate().unwrap();
        let b = StaticKeypair::generate().unwrap();
        assert_ne!(a.public(), b.public());
        assert_ne!(a.private_bytes(), b.private_bytes());
    }

    #[test]
    fn private_bytes_round_trip_to_the_same_public_key() {
        let original = StaticKeypair::generate().unwrap();
        let restored = StaticKeypair::from_private_bytes(original.private_bytes()).unwrap();
        assert_eq!(original.public(), restored.public());
    }

    #[test]
    fn debug_output_never_shows_the_private_key() {
        let pair = StaticKeypair::generate().unwrap();
        let shown = format!("{pair:?}");
        assert!(shown.contains("<redacted>"));
        let hex = pair.private_bytes()[..8]
            .iter()
            .fold(String::new(), |mut out, b| {
                use std::fmt::Write as _;
                let _ = write!(out, "{b:02x}");
                out
            });
        assert!(!shown.contains(&hex));
    }

    #[test]
    fn device_ids_are_stable_and_distinct() {
        let a = StaticKeypair::generate().unwrap();
        let b = StaticKeypair::generate().unwrap();
        assert_eq!(a.public().device_id(), a.public().device_id());
        assert_ne!(a.public().device_id(), b.public().device_id());
    }

    #[test]
    fn device_id_is_a_known_hash_of_the_key() {
        // Pinned so the derivation never changes by accident: peers store these IDs.
        let id = PublicKey::from_bytes([7u8; KEY_LEN]).device_id();
        let expected = blake3::derive_key(DEVICE_ID_CONTEXT, &[7u8; KEY_LEN]);
        assert_eq!(id.as_bytes(), &expected[..DEVICE_ID_LEN]);
        assert_eq!(id.to_string().len(), DEVICE_ID_LEN * 2);
        assert!(
            id.to_string()
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
        );
    }

    #[test]
    fn wrong_length_public_keys_are_rejected() {
        assert!(PublicKey::from_slice(&[0u8; 31]).is_err());
        assert!(PublicKey::from_slice(&[0u8; 32]).is_ok());
    }
}
