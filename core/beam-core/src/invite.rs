//! The pairing invite: what a QR code carries so another device can pair with this one.
//!
//! Text form: `BEAM1:` followed by base32 of the binary form:
//!
//! ```text
//! version u8 = 1 | cipher u8 | token [16] | displayer key [32] | hint count u8 (≤ 4)
//! hint = family u8 (4 or 6) | address [4 or 16] | port u16 BE
//! ```
//!
//! The token is single-use and short-lived; the displayer enforces both. The signaling room
//! is derived from the token, so it costs no space in the code.

use std::net::SocketAddr;

use crate::handshake::MIN_PAIRING_TOKEN_LEN;
use crate::{BeamError, Cipher, PublicKey, Result, TransferId, base32, hints};

/// Every invite starts with this; it tells a scanner it's a Beam code and which format.
pub const INVITE_PREFIX: &str = "BEAM1:";

pub use crate::hints::MAX_HINTS;

const VERSION: u8 = 1;
const TOKEN_LEN: usize = MIN_PAIRING_TOKEN_LEN;
const ROOM_CONTEXT: &str = "beam 2026-11-25 pairing room v1";

/// An invitation to pair with the device that displays it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PairingInvite {
    /// One-time secret that binds the handshake to this invite.
    pub token: [u8; TOKEN_LEN],
    /// The displaying device's public key; the scanner accepts no other.
    pub displayer: PublicKey,
    /// The session cipher both devices will use.
    pub cipher: Cipher,
    /// Where the displayer accepts connections; empty when it can't (a browser).
    pub hints: Vec<SocketAddr>,
}

impl PairingInvite {
    /// A new invite with a fresh random token.
    ///
    /// # Errors
    ///
    /// [`BeamError::Malformed`] for more than [`MAX_HINTS`] hints or a hint with port 0, and
    /// [`BeamError::Handshake`] if the random number generator fails.
    pub fn new(displayer: PublicKey, cipher: Cipher, hints: Vec<SocketAddr>) -> Result<Self> {
        hints::check(&hints)?;
        Ok(Self {
            token: TransferId::random()?.0,
            displayer,
            cipher,
            hints,
        })
    }

    /// The same invite (same token) with other connection hints, for when the displayer's
    /// addresses change while its code is on screen.
    ///
    /// # Errors
    ///
    /// [`BeamError::Malformed`] for more than [`MAX_HINTS`] hints or a hint with port 0.
    pub fn with_hints(&self, hints: Vec<SocketAddr>) -> Result<Self> {
        hints::check(&hints)?;
        Ok(Self {
            hints,
            ..self.clone()
        })
    }

    /// The signaling room both devices meet in when a direct connection fails.
    #[must_use]
    pub fn room(&self) -> [u8; 16] {
        let key = blake3::derive_key(ROOM_CONTEXT, &self.token);
        let mut room = [0u8; 16];
        room.copy_from_slice(&key[..16]);
        room
    }

    /// The text to put in the QR code.
    #[must_use]
    pub fn encode(&self) -> String {
        let mut bytes = vec![VERSION, cipher_code(self.cipher)];
        bytes.extend_from_slice(&self.token);
        bytes.extend_from_slice(self.displayer.as_bytes());
        // At most MAX_HINTS: `new` and `decode` refuse more, and the fields are only read here.
        hints::encode(&self.hints, &mut bytes);
        format!("{INVITE_PREFIX}{}", base32::encode(&bytes))
    }

    /// Reads a scanned code.
    ///
    /// # Errors
    ///
    /// [`BeamError::Malformed`] if it isn't a Beam invite, has an unknown version, or is
    /// damaged or truncated.
    pub fn decode(text: &str) -> Result<Self> {
        let body = text
            .trim()
            .strip_prefix(INVITE_PREFIX)
            .ok_or_else(|| malformed("not a Beam pairing code"))?;
        let bytes = base32::decode(body).ok_or_else(|| malformed("damaged pairing code"))?;
        let mut reader = Reader(&bytes);
        if reader.byte()? != VERSION {
            return Err(malformed("pairing code from a newer version of Beam"));
        }
        let cipher = match reader.byte()? {
            0 => Cipher::AesGcm,
            1 => Cipher::ChaChaPoly,
            _ => return Err(malformed("unknown cipher in pairing code")),
        };
        let token = reader.array()?;
        let displayer = PublicKey::from_bytes(reader.array()?);
        let (hints, rest) = hints::decode(reader.0)?;
        if !rest.is_empty() {
            return Err(malformed("trailing bytes in pairing code"));
        }
        Ok(Self {
            token,
            displayer,
            cipher,
            hints,
        })
    }
}

const fn cipher_code(cipher: Cipher) -> u8 {
    match cipher {
        Cipher::AesGcm => 0,
        Cipher::ChaChaPoly => 1,
    }
}

fn malformed(why: &str) -> BeamError {
    BeamError::Malformed(why.into())
}

struct Reader<'a>(&'a [u8]);

impl Reader<'_> {
    fn byte(&mut self) -> Result<u8> {
        Ok(self.array::<1>()?[0])
    }

    fn array<const N: usize>(&mut self) -> Result<[u8; N]> {
        let (head, rest) = self
            .0
            .split_first_chunk::<N>()
            .ok_or_else(|| malformed("truncated pairing code"))?;
        self.0 = rest;
        Ok(*head)
    }
}

#[cfg(test)]
#[path = "invite_tests.rs"]
mod tests;
