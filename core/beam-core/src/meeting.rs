//! Meeting rooms for devices that already trust each other (see `protocol.md`, Meeting
//! again): where a browser and the phone it paired with find each other later, without a
//! pairing code.
//!
//! Both devices hold their own key pair and the other's public key, so both compute the same
//! X25519 secret, which nobody else can, and derive from it, with both public keys:
//!
//! ```text
//! ikm  = X25519(own private, peer public) | lower public key | higher public key
//! room = BLAKE3-derive_key("beam 2026-11-27 meeting room v1", ikm)[..16]
//! seal = BLAKE3-derive_key("beam 2026-11-27 meeting seal v1", ikm)   (a SignalSeal key)
//! ```
//!
//! The room is the same every time the two meet, so the signaling service can tell that the
//! same pair met again (it can't tell who they are, or read what they say). Messages are
//! sealed as in pairing; the phone, which serves, seals as [`crate::SignalFrom::Scanner`] and
//! the browser as [`crate::SignalFrom::Displayer`].

use zeroize::Zeroizing;

use crate::{PublicKey, Result, SignalSeal, StaticKeypair};

const ROOM_CONTEXT: &str = "beam 2026-11-27 meeting room v1";
const SEAL_CONTEXT: &str = "beam 2026-11-27 meeting seal v1";

/// Where two trusted devices meet, and how they seal what they say there.
pub struct Meeting {
    room: [u8; 16],
    seal: SignalSeal,
}

impl Meeting {
    /// The meeting between this device (`local`) and the trusted device whose key is `peer`.
    ///
    /// # Errors
    ///
    /// [`crate::BeamError::Handshake`] if `peer` is a weak key.
    pub fn between(local: &StaticKeypair, peer: &PublicKey) -> Result<Self> {
        let shared = local.agree(peer)?;
        let own = local.public();
        let (low, high) = if own.as_bytes() <= peer.as_bytes() {
            (own, *peer)
        } else {
            (*peer, own)
        };
        let mut ikm = Zeroizing::new([0u8; 96]);
        ikm[..32].copy_from_slice(shared.as_slice());
        ikm[32..64].copy_from_slice(low.as_bytes());
        ikm[64..].copy_from_slice(high.as_bytes());

        let mut room = [0u8; 16];
        room.copy_from_slice(&blake3::derive_key(ROOM_CONTEXT, ikm.as_slice())[..16]);
        let key = Zeroizing::new(blake3::derive_key(SEAL_CONTEXT, ikm.as_slice()));
        Ok(Self {
            room,
            seal: SignalSeal::with_key(&key)?,
        })
    }

    /// The signaling room both devices join.
    #[must_use]
    pub const fn room(&self) -> [u8; 16] {
        self.room
    }

    /// Seals and opens what they post there.
    #[must_use]
    pub const fn seal(&self) -> &SignalSeal {
        &self.seal
    }
}

#[cfg(test)]
#[path = "meeting_tests.rs"]
mod tests;
