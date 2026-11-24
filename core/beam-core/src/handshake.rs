//! Noise handshakes that authenticate two devices and derive session keys.
//!
//! - **Pairing (XX):** the first connection. One device displays a QR code with a one-time token
//!   and its public key; the device with a camera (normally the phone) scans it. Who opens the
//!   connection is a separate [`Role`]. The prologue binds the token and the displayed key, so a
//!   relay or signaling server in the middle cannot splice two handshakes together, and the
//!   scanner rejects any static key other than the displayed one the moment it arrives.
//! - **Known device (IK):** the initiator already trusts the responder's key, so the handshake
//!   takes one round trip.
//!
//! The module does no I/O: the caller moves the bytes from [`Handshake::write_message`] to the
//! peer and feeds replies to [`Handshake::read_message`] until [`Handshake::is_finished`].

use snow::{Builder, HandshakeState, TransportState};

use crate::keys::{PublicKey, StaticKeypair};
use crate::verification::VerificationCode;
use crate::{BeamError, Result};

/// Shortest accepted pairing token, in bytes.
pub const MIN_PAIRING_TOKEN_LEN: usize = 16;
/// Longest accepted pairing token, in bytes.
pub const MAX_PAIRING_TOKEN_LEN: usize = 64;

const MAX_MESSAGE_LEN: usize = 65_535;
const PAIRING_PROLOGUE: &[u8] = b"beam/1 pairing";
const KNOWN_DEVICE_PROLOGUE: &[u8] = b"beam/1 known-device";

/// Symmetric cipher for the session. Both peers must pick the same one before connecting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Cipher {
    /// AES-256-GCM, preferred where the CPU accelerates AES.
    AesGcm,
    /// ChaCha20-Poly1305, the fallback for devices without AES acceleration.
    ChaChaPoly,
}

impl Cipher {
    const fn noise_name(self) -> &'static str {
        match self {
            Self::AesGcm => "AESGCM",
            Self::ChaChaPoly => "ChaChaPoly",
        }
    }
}

/// Which side opens the connection. Independent of who shows the QR code: a browser always
/// connects out (it cannot accept connections), whether it displayed the code or not.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Role {
    /// Opens the connection and sends the first handshake message.
    Initiator,
    /// Accepts the connection.
    Responder,
}

/// An in-progress handshake.
pub struct Handshake {
    state: HandshakeState,
    expected_peer: Option<PublicKey>,
}

impl Handshake {
    /// Starts pairing as the device that scanned the QR code (normally a phone), using the token
    /// and public key it read. `role` says whether this side opens the connection.
    ///
    /// # Errors
    ///
    /// Returns [`BeamError::Handshake`] if the token length is outside the accepted range.
    pub fn pairing_scanner(
        local: &StaticKeypair,
        cipher: Cipher,
        role: Role,
        token: &[u8],
        displayed_key: PublicKey,
    ) -> Result<Self> {
        let prologue = pairing_prologue(token, displayed_key)?;
        Ok(Self {
            state: pairing_state(local, cipher, role, &prologue)?,
            expected_peer: Some(displayed_key),
        })
    }

    /// Starts pairing as the device that displayed `token` and its own key in a QR code
    /// (normally a laptop or browser). `role` says whether this side opens the connection.
    ///
    /// # Errors
    ///
    /// Returns [`BeamError::Handshake`] if the token length is outside the accepted range.
    pub fn pairing_displayer(
        local: &StaticKeypair,
        cipher: Cipher,
        role: Role,
        token: &[u8],
    ) -> Result<Self> {
        let prologue = pairing_prologue(token, local.public())?;
        Ok(Self {
            state: pairing_state(local, cipher, role, &prologue)?,
            expected_peer: None,
        })
    }

    /// Connects to an already trusted device whose public key is `peer`.
    ///
    /// # Errors
    ///
    /// Returns [`BeamError::Handshake`] if the handshake cannot be set up.
    pub fn known_initiator(local: &StaticKeypair, cipher: Cipher, peer: PublicKey) -> Result<Self> {
        let state = builder("IK", cipher)?
            .local_private_key(local.private_bytes())
            .and_then(|b| b.remote_public_key(peer.as_bytes()))
            .and_then(|b| b.prologue(KNOWN_DEVICE_PROLOGUE))
            .and_then(Builder::build_initiator)
            .map_err(noise_error)?;
        Ok(Self {
            state,
            expected_peer: Some(peer),
        })
    }

    /// Accepts a connection from a device that claims to be trusted. Check
    /// [`Established::peer`] against the trusted-device list before using the session.
    ///
    /// # Errors
    ///
    /// Returns [`BeamError::Handshake`] if the handshake cannot be set up.
    pub fn known_responder(local: &StaticKeypair, cipher: Cipher) -> Result<Self> {
        let state = builder("IK", cipher)?
            .local_private_key(local.private_bytes())
            .and_then(|b| b.prologue(KNOWN_DEVICE_PROLOGUE))
            .and_then(Builder::build_responder)
            .map_err(noise_error)?;
        Ok(Self {
            state,
            expected_peer: None,
        })
    }

    /// Whether the next step is ours to write.
    #[must_use]
    pub fn is_my_turn(&self) -> bool {
        self.state.is_my_turn()
    }

    /// Whether all handshake messages have been exchanged.
    #[must_use]
    pub fn is_finished(&self) -> bool {
        self.state.is_handshake_finished()
    }

    /// Produces the next handshake message to send to the peer.
    ///
    /// # Errors
    ///
    /// Returns [`BeamError::Handshake`] if it is not our turn or the handshake is over.
    pub fn write_message(&mut self) -> Result<Vec<u8>> {
        let mut message = vec![0u8; MAX_MESSAGE_LEN];
        let len = self
            .state
            .write_message(&[], &mut message)
            .map_err(noise_error)?;
        message.truncate(len);
        Ok(message)
    }

    /// Consumes a handshake message from the peer.
    ///
    /// # Errors
    ///
    /// Returns [`BeamError::Handshake`] if the message fails authentication or arrives out of
    /// turn, [`BeamError::Malformed`] if it carries an unexpected payload, and
    /// [`BeamError::UnexpectedPeerKey`] if the peer is not the device we expected.
    pub fn read_message(&mut self, message: &[u8]) -> Result<()> {
        let mut payload = vec![0u8; MAX_MESSAGE_LEN];
        let len = self
            .state
            .read_message(message, &mut payload)
            .map_err(noise_error)?;
        if len != 0 {
            return Err(BeamError::Malformed(
                "handshake message carries a payload".into(),
            ));
        }
        self.check_peer()
    }

    /// Completes the handshake and returns the authenticated session.
    ///
    /// # Errors
    ///
    /// Returns [`BeamError::Handshake`] if messages are still outstanding and
    /// [`BeamError::UnexpectedPeerKey`] if the peer is not the device we expected.
    pub fn finish(self) -> Result<Established> {
        if !self.is_finished() {
            return Err(BeamError::Handshake("handshake is not finished".into()));
        }
        self.check_peer()?;
        let peer = self
            .state
            .get_remote_static()
            .ok_or_else(|| BeamError::Handshake("peer sent no static key".into()))
            .and_then(PublicKey::from_slice)?;
        let code = VerificationCode::derive(self.state.get_handshake_hash());
        let transport = self.state.into_transport_mode().map_err(noise_error)?;
        Ok(Established {
            peer,
            code,
            transport,
        })
    }

    fn check_peer(&self) -> Result<()> {
        let (Some(expected), Some(actual)) = (self.expected_peer, self.state.get_remote_static())
        else {
            return Ok(());
        };
        if actual == expected.as_bytes() {
            Ok(())
        } else {
            Err(BeamError::UnexpectedPeerKey)
        }
    }
}

/// The result of a completed handshake: an authenticated peer and session keys.
pub struct Established {
    peer: PublicKey,
    code: VerificationCode,
    transport: TransportState,
}

impl Established {
    /// The peer's authenticated static public key.
    #[must_use]
    pub const fn peer(&self) -> PublicKey {
        self.peer
    }

    /// The code both users can compare to confirm a pairing.
    #[must_use]
    pub const fn verification_code(&self) -> VerificationCode {
        self.code
    }

    /// Hands the session keys to the encrypted session layer.
    pub(crate) fn into_transport(self) -> TransportState {
        self.transport
    }
}

fn builder<'a>(pattern: &str, cipher: Cipher) -> Result<Builder<'a>> {
    let name = format!("Noise_{pattern}_25519_{}_BLAKE2s", cipher.noise_name());
    let params = name.parse().map_err(noise_error)?;
    Ok(Builder::new(params))
}

fn pairing_state(
    local: &StaticKeypair,
    cipher: Cipher,
    role: Role,
    prologue: &[u8],
) -> Result<HandshakeState> {
    let builder = builder("XX", cipher)?
        .local_private_key(local.private_bytes())
        .and_then(|b| b.prologue(prologue))
        .map_err(noise_error)?;
    match role {
        Role::Initiator => builder.build_initiator(),
        Role::Responder => builder.build_responder(),
    }
    .map_err(noise_error)
}

fn pairing_prologue(token: &[u8], phone_key: PublicKey) -> Result<Vec<u8>> {
    let len = u8::try_from(token.len())
        .ok()
        .filter(|_| (MIN_PAIRING_TOKEN_LEN..=MAX_PAIRING_TOKEN_LEN).contains(&token.len()))
        .ok_or_else(|| {
            BeamError::Handshake(format!(
                "pairing token must be {MIN_PAIRING_TOKEN_LEN} to {MAX_PAIRING_TOKEN_LEN} bytes"
            ))
        })?;
    let mut prologue = Vec::with_capacity(PAIRING_PROLOGUE.len() + 1 + token.len() + 32);
    prologue.extend_from_slice(PAIRING_PROLOGUE);
    prologue.push(len);
    prologue.extend_from_slice(token);
    prologue.extend_from_slice(phone_key.as_bytes());
    Ok(prologue)
}

#[allow(clippy::needless_pass_by_value)] // used as `map_err(noise_error)`
fn noise_error(err: snow::Error) -> BeamError {
    BeamError::Handshake(err.to_string())
}

#[cfg(test)]
#[path = "handshake_tests.rs"]
mod tests;
