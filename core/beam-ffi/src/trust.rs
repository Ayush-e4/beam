//! The app's answer to "is this a trusted device?", asked for every incoming offer.

/// The app's trusted devices, as the engine sees them. Called on an engine thread for each
/// incoming offer; keep it quick (an in-memory lookup).
#[uniffi::export(with_foreign)]
pub trait TrustedKeys: Send + Sync {
    /// Whether the device with this public key is one this device paired with.
    fn is_trusted(&self, public_key: Vec<u8>) -> bool;
}
