//! The device's long-term identity, as the app sees it.
//!
//! The private key lives in Rust memory that is wiped on drop. Kotlin only touches its bytes
//! to wrap them with an Android Keystore key for storage ([`DeviceIdentity::private_key`]) and
//! to hand them back after unwrapping ([`DeviceIdentity::from_private_key`]).

use std::sync::Arc;

use beam_core::StaticKeypair;
use beam_core::keys::KEY_LEN;

use crate::BeamError;

/// This device's X25519 key pair and the device ID derived from it.
#[derive(Debug, uniffi::Object)]
pub struct DeviceIdentity {
    keypair: StaticKeypair,
}

#[uniffi::export]
impl DeviceIdentity {
    /// Creates a new random identity, for the first launch.
    ///
    /// # Errors
    ///
    /// [`BeamError::Crypto`] if the system random number generator fails.
    #[uniffi::constructor]
    pub fn generate() -> Result<Arc<Self>, BeamError> {
        Ok(Arc::new(Self {
            keypair: StaticKeypair::generate()?,
        }))
    }

    /// Restores a stored identity from its 32-byte private key.
    ///
    /// # Errors
    ///
    /// [`BeamError::Crypto`] if the key is not 32 bytes.
    #[uniffi::constructor]
    pub fn from_private_key(private_key: Vec<u8>) -> Result<Arc<Self>, BeamError> {
        let private_key = zeroize::Zeroizing::new(private_key);
        let bytes: &[u8; KEY_LEN] =
            private_key
                .as_slice()
                .try_into()
                .map_err(|_| BeamError::Crypto {
                    detail: format!("a device private key is {KEY_LEN} bytes"),
                })?;
        Ok(Arc::new(Self {
            keypair: StaticKeypair::from_private_bytes(bytes)?,
        }))
    }

    /// The 32-byte public key, shared with peers when pairing.
    #[must_use]
    pub fn public_key(&self) -> Vec<u8> {
        self.keypair.public().as_bytes().to_vec()
    }

    /// The device ID: 32 lowercase hex characters derived from the public key.
    #[must_use]
    pub fn device_id(&self) -> String {
        self.keypair.public().device_id().to_string()
    }

    /// The 32-byte private key, only for wrapping it with a Keystore key before storing it.
    /// Overwrite the returned array once it has been wrapped.
    #[must_use]
    pub fn private_key(&self) -> Vec<u8> {
        self.keypair.private_bytes().to_vec()
    }
}

impl DeviceIdentity {
    /// The key pair, for the engine's handshakes.
    #[must_use]
    pub fn keypair(&self) -> &StaticKeypair {
        &self.keypair
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_stored_identity_restores_the_same_device() {
        let original = DeviceIdentity::generate().unwrap();
        let restored = DeviceIdentity::from_private_key(original.private_key()).unwrap();
        assert_eq!(restored.public_key(), original.public_key());
        assert_eq!(restored.device_id(), original.device_id());
    }

    #[test]
    fn new_identities_differ() {
        let a = DeviceIdentity::generate().unwrap();
        let b = DeviceIdentity::generate().unwrap();
        assert_ne!(a.device_id(), b.device_id());
    }

    #[test]
    fn a_key_of_the_wrong_length_is_a_crypto_error() {
        let error = DeviceIdentity::from_private_key(vec![1; 31]).unwrap_err();
        assert!(matches!(error, BeamError::Crypto { .. }), "{error:?}");
    }

    #[test]
    fn debug_output_hides_the_private_key() {
        let identity = DeviceIdentity::generate().unwrap();
        assert!(format!("{identity:?}").contains("<redacted>"));
    }
}
