//! Sealed signaling messages: what two pairing devices pass each other through a signaling
//! room (see `protocol.md`, Signaling rooms) when they can't connect directly yet.
//!
//! The room's server relays them but can't read, change or forge them: each is sealed with a
//! key derived from the pairing token, which only the two devices know.
//!
//! ```text
//! sealed  = version u8 (1) | nonce [12] | ChaCha20-Poly1305(key, nonce, aad, message)
//! aad     = "beam signal v1" | version u8 | from u8 (0 displayer, 1 scanner)
//! message = type u8 | body
//!   1 offer: hints | certificate SHA-256 [32]
//! ```
//!
//! `from` in the AAD means a message only opens as coming from the side that sealed it, so the
//! server can't bounce a device's own message back to it.

use std::net::SocketAddr;

use ring::aead::{Aad, CHACHA20_POLY1305, LessSafeKey, NONCE_LEN, Nonce, UnboundKey};
use snow::resolvers::{CryptoResolver, DefaultResolver};
use zeroize::Zeroizing;

use crate::{BeamError, PairingInvite, Result, hints};

const VERSION: u8 = 1;
const KEY_CONTEXT: &str = "beam 2026-11-26 signaling seal v1";
const AAD_LABEL: &[u8] = b"beam signal v1";
const OFFER: u8 = 1;

/// Which side of a pairing sealed a message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignalFrom {
    /// The device showing the pairing code (usually the browser).
    Displayer,
    /// The device that scanned it (usually the phone).
    Scanner,
}

/// What one device tells the other through the room.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SignalMessage {
    /// Where the sender accepts connections, and the SHA-256 of its WebTransport certificate,
    /// which a browser pins instead of trusting a certificate authority.
    Offer {
        /// Addresses and ports (at most [`hints::MAX_HINTS`]).
        hints: Vec<SocketAddr>,
        /// SHA-256 of the certificate the sender's WebTransport server presents.
        certificate_hash: [u8; 32],
    },
}

/// Seals and opens one pairing's signaling messages.
pub struct SignalSeal {
    key: LessSafeKey,
}

impl SignalSeal {
    /// The seal for the pairing that `invite` starts.
    ///
    /// # Errors
    ///
    /// [`BeamError::Handshake`] if the cipher can't be set up (it always can).
    pub fn for_invite(invite: &PairingInvite) -> Result<Self> {
        Self::with_key(&Zeroizing::new(blake3::derive_key(
            KEY_CONTEXT,
            &invite.token,
        )))
    }

    /// A seal under `key`, which the caller derived for this purpose alone.
    pub(crate) fn with_key(key: &[u8; 32]) -> Result<Self> {
        let key = UnboundKey::new(&CHACHA20_POLY1305, key)
            .map_err(|_| BeamError::Handshake("signaling key unusable".into()))?;
        Ok(Self {
            key: LessSafeKey::new(key),
        })
    }

    /// Seals `message`, sent by `from`.
    ///
    /// # Errors
    ///
    /// [`BeamError::Malformed`] for an invalid message (too many hints, a hint without a
    /// port), and [`BeamError::Handshake`] if the random number generator fails.
    pub fn seal(&self, from: SignalFrom, message: &SignalMessage) -> Result<Vec<u8>> {
        let mut body = encode(message)?;
        let nonce = random_nonce()?;
        self.key
            .seal_in_place_append_tag(Nonce::assume_unique_for_key(nonce), aad(from), &mut body)
            .map_err(|_| BeamError::Handshake("signaling message too long to seal".into()))?;
        let mut sealed = Vec::with_capacity(1 + NONCE_LEN + body.len());
        sealed.push(VERSION);
        sealed.extend_from_slice(&nonce);
        sealed.extend_from_slice(&body);
        Ok(sealed)
    }

    /// Opens a message that `from` sealed.
    ///
    /// # Errors
    ///
    /// [`BeamError::Handshake`] if it wasn't sealed with this pairing's key by `from`, or was
    /// changed on the way; [`BeamError::Malformed`] if it is from a newer version or damaged.
    pub fn open(&self, from: SignalFrom, sealed: &[u8]) -> Result<SignalMessage> {
        let (&version, rest) = sealed
            .split_first()
            .ok_or_else(|| malformed("empty signaling message"))?;
        if version != VERSION {
            return Err(malformed("signaling message from a newer version of Beam"));
        }
        let (nonce, body) = rest
            .split_first_chunk::<NONCE_LEN>()
            .ok_or_else(|| malformed("truncated signaling message"))?;
        let mut body = body.to_vec();
        let plain = self
            .key
            .open_in_place(Nonce::assume_unique_for_key(*nonce), aad(from), &mut body)
            .map_err(|_| BeamError::Handshake("signaling message not sealed for us".into()))?;
        decode(plain)
    }
}

fn aad(from: SignalFrom) -> Aad<[u8; AAD_LABEL.len() + 2]> {
    let mut aad = [0u8; AAD_LABEL.len() + 2];
    aad[..AAD_LABEL.len()].copy_from_slice(AAD_LABEL);
    aad[AAD_LABEL.len()] = VERSION;
    aad[AAD_LABEL.len() + 1] = match from {
        SignalFrom::Displayer => 0,
        SignalFrom::Scanner => 1,
    };
    Aad::from(aad)
}

/// A fresh random nonce: each pairing seals a handful of messages, so random 96-bit nonces
/// never repeat in practice.
fn random_nonce() -> Result<[u8; NONCE_LEN]> {
    let mut rng = DefaultResolver
        .resolve_rng()
        .ok_or_else(|| BeamError::Handshake("no random number generator".into()))?;
    let mut nonce = [0u8; NONCE_LEN];
    rng.try_fill_bytes(&mut nonce)
        .map_err(|err| BeamError::Handshake(format!("random number generator failed: {err}")))?;
    Ok(nonce)
}

/// The plain message, before sealing.
///
/// # Errors
///
/// [`BeamError::Malformed`] for too many hints or a hint without a port.
pub fn encode(message: &SignalMessage) -> Result<Vec<u8>> {
    match message {
        SignalMessage::Offer {
            hints: offered,
            certificate_hash,
        } => {
            hints::check(offered)?;
            let mut bytes = vec![OFFER];
            hints::encode(offered, &mut bytes);
            bytes.extend_from_slice(certificate_hash);
            Ok(bytes)
        }
    }
}

/// Reads a plain message (after opening).
///
/// # Errors
///
/// [`BeamError::Malformed`] for an unknown type or a damaged or truncated body.
pub fn decode(bytes: &[u8]) -> Result<SignalMessage> {
    let (&kind, body) = bytes
        .split_first()
        .ok_or_else(|| malformed("empty signaling message"))?;
    match kind {
        OFFER => {
            let (offered, rest) = hints::decode(body)?;
            let certificate_hash = <[u8; 32]>::try_from(rest)
                .map_err(|_| malformed("signaling offer without a certificate hash"))?;
            Ok(SignalMessage::Offer {
                hints: offered,
                certificate_hash,
            })
        }
        _ => Err(malformed("unknown signaling message")),
    }
}

fn malformed(why: &str) -> BeamError {
    BeamError::Malformed(why.into())
}

#[cfg(test)]
#[path = "signaling_tests.rs"]
mod tests;
