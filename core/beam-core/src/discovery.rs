//! Discovery beacons: what a device announces on the local network so that only the devices it
//! paired with can recognise it.
//!
//! Each device has a secret [`DiscoveryKey`], handed to a peer only inside the encrypted
//! pairing `HELLO`. It announces a [`Beacon`]: a fresh random nonce and a keyed BLAKE3 tag of
//! that nonce. A trusted peer recognises it by checking the tag against the keys it holds;
//! everyone else sees random bytes that change whenever the device picks a new nonce.
//!
//! ```text
//! Beacon = nonce (8 bytes) | tag (8 bytes)      tag = BLAKE3-keyed(key, "beam beacon" | nonce)
//! ```

use snow::resolvers::{CryptoResolver, DefaultResolver};
use zeroize::Zeroizing;

use crate::keys::{KEY_LEN, StaticKeypair};
use crate::{BeamError, Result, base32};

/// Bytes in a discovery key.
pub const DISCOVERY_KEY_LEN: usize = 32;
const NONCE_LEN: usize = 8;
const TAG_LEN: usize = 8;
const BEACON_LEN: usize = NONCE_LEN + TAG_LEN;
const KEY_CONTEXT: &str = "beam 2026-11-25 discovery key v1";
const TAG_DOMAIN: &[u8] = b"beam beacon";

/// The secret a device's trusted peers use to recognise its beacons.
#[derive(Clone, PartialEq, Eq)]
pub struct DiscoveryKey(Zeroizing<[u8; DISCOVERY_KEY_LEN]>);

impl std::fmt::Debug for DiscoveryKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("DiscoveryKey(..)")
    }
}

impl DiscoveryKey {
    /// This device's key for `generation`. Moving to a new generation (after removing a
    /// trusted device) makes beacons unrecognisable to anyone holding only older keys.
    #[must_use]
    pub fn derive(identity: &StaticKeypair, generation: u32) -> Self {
        let mut material = Zeroizing::new([0u8; KEY_LEN + 4]);
        material[..KEY_LEN].copy_from_slice(identity.private_bytes());
        material[KEY_LEN..].copy_from_slice(&generation.to_le_bytes());
        Self(Zeroizing::new(blake3::derive_key(KEY_CONTEXT, &*material)))
    }

    /// A key received from a peer.
    ///
    /// # Errors
    ///
    /// [`BeamError::Malformed`] unless `bytes` is exactly [`DISCOVERY_KEY_LEN`] long.
    pub fn from_slice(bytes: &[u8]) -> Result<Self> {
        let key: [u8; DISCOVERY_KEY_LEN] = bytes
            .try_into()
            .map_err(|_| BeamError::Malformed("discovery key has the wrong length".into()))?;
        Ok(Self(Zeroizing::new(key)))
    }

    /// The raw key, for storing with a trusted device.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8; DISCOVERY_KEY_LEN] {
        &self.0
    }

    fn tag(&self, nonce: [u8; NONCE_LEN]) -> [u8; TAG_LEN] {
        let mut hasher = blake3::Hasher::new_keyed(&self.0);
        hasher.update(TAG_DOMAIN).update(&nonce);
        let mut tag = [0u8; TAG_LEN];
        tag.copy_from_slice(&hasher.finalize().as_bytes()[..TAG_LEN]);
        tag
    }
}

/// One announcement: a nonce and its tag. Pick a new one regularly and whenever the network
/// changes, so the same beacon is never seen across places or for long.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Beacon {
    nonce: [u8; NONCE_LEN],
    tag: [u8; TAG_LEN],
}

impl Beacon {
    /// A new beacon for `key` with a fresh random nonce.
    ///
    /// # Errors
    ///
    /// [`BeamError::Handshake`] if the random number generator fails.
    pub fn new(key: &DiscoveryKey) -> Result<Self> {
        let mut rng = DefaultResolver
            .resolve_rng()
            .ok_or_else(|| BeamError::Handshake("no random number generator".into()))?;
        let mut nonce = [0u8; NONCE_LEN];
        rng.try_fill_bytes(&mut nonce).map_err(|err| {
            BeamError::Handshake(format!("random number generator failed: {err}"))
        })?;
        Ok(Self::with_nonce(key, nonce))
    }

    fn with_nonce(key: &DiscoveryKey, nonce: [u8; NONCE_LEN]) -> Self {
        Self {
            nonce,
            tag: key.tag(nonce),
        }
    }

    /// The beacon as 26 upper-case base32 characters, for an mDNS instance name or TXT value.
    #[must_use]
    pub fn encode(&self) -> String {
        let mut bytes = [0u8; BEACON_LEN];
        bytes[..NONCE_LEN].copy_from_slice(&self.nonce);
        bytes[NONCE_LEN..].copy_from_slice(&self.tag);
        base32::encode(&bytes)
    }

    /// Reads an announced beacon. Case-insensitive, since some mDNS stacks change case.
    ///
    /// # Errors
    ///
    /// [`BeamError::Malformed`] if `text` isn't a beacon.
    pub fn parse(text: &str) -> Result<Self> {
        let not_a_beacon = || BeamError::Malformed("not a Beam beacon".into());
        let bytes = base32::decode(&text.to_ascii_uppercase()).ok_or_else(not_a_beacon)?;
        let bytes: [u8; BEACON_LEN] = bytes.try_into().map_err(|_| not_a_beacon())?;
        let mut beacon = Self {
            nonce: [0; NONCE_LEN],
            tag: [0; TAG_LEN],
        };
        beacon.nonce.copy_from_slice(&bytes[..NONCE_LEN]);
        beacon.tag.copy_from_slice(&bytes[NONCE_LEN..]);
        Ok(beacon)
    }

    /// Whether this beacon was made with `key`. Compares in constant time.
    #[must_use]
    pub fn matches(&self, key: &DiscoveryKey) -> bool {
        let expected = key.tag(self.nonce);
        expected
            .iter()
            .zip(self.tag)
            .fold(0u8, |diff, (a, b)| diff | (a ^ b))
            == 0
    }

    /// The index of the first key in `keys` that made this beacon, if any.
    pub fn find<'a, I>(&self, keys: I) -> Option<usize>
    where
        I: IntoIterator<Item = &'a DiscoveryKey>,
    {
        keys.into_iter().position(|key| self.matches(key))
    }
}

#[cfg(test)]
#[path = "discovery_tests.rs"]
mod tests;
