//! Beam's engine in the browser, compiled to WebAssembly.
//!
//! The web app opens a WebTransport session to the phone and hands this crate the session's
//! bidirectional stream. Everything above the bytes (the Noise handshake, the encrypted session,
//! frames) is beam-core, the same code the phone runs.

mod errors;
mod js_stream;
pub mod opfs_file;
pub mod opfs_staging;
mod pairing;
mod progress;
mod receiving;
mod sending;

use beam_core::{
    Cipher, Connection, Frame, FrameType, Handshake, PairingInvite, PublicKey, Role, StaticKeypair,
};
use wasm_bindgen::prelude::*;
use web_sys::{ReadableStream, WritableStream};

use crate::errors::{beam_error, invalid};
use crate::js_stream::JsStream;

/// Control stream: frames that aren't part of a transfer.
const CONTROL_STREAM: u32 = 0;

/// Reads a cipher name: `"aes-gcm"` or `"chacha-poly"`. The device that shows the pairing
/// code chooses; in a browser ChaCha20-Poly1305 is faster, as WebAssembly has no AES
/// instructions.
fn cipher(name: &str) -> Result<Cipher, JsValue> {
    match name {
        "aes-gcm" => Ok(Cipher::AesGcm),
        "chacha-poly" => Ok(Cipher::ChaChaPoly),
        _ => Err(invalid("cipher must be \"aes-gcm\" or \"chacha-poly\"")),
    }
}

/// This browser's long-term key pair. Store [`Identity::private_key`] somewhere only this
/// site can read (`IndexedDB`), so paired phones recognise the browser next time.
#[wasm_bindgen]
pub struct Identity {
    pub(crate) keypair: StaticKeypair,
}

#[wasm_bindgen]
impl Identity {
    /// A new random identity.
    ///
    /// # Errors
    ///
    /// If the browser's random number generator fails.
    pub fn generate() -> Result<Self, JsValue> {
        let keypair = StaticKeypair::generate().map_err(beam_error)?;
        Ok(Self { keypair })
    }

    /// The identity saved earlier from [`Identity::private_key`].
    ///
    /// # Errors
    ///
    /// If `private_key` isn't 32 bytes or isn't a valid key.
    #[wasm_bindgen(js_name = fromPrivateKey)]
    pub fn from_private_key(private_key: &[u8]) -> Result<Self, JsValue> {
        let bytes: [u8; 32] = private_key
            .try_into()
            .map_err(|_| invalid("a private key is 32 bytes"))?;
        let keypair = StaticKeypair::from_private_bytes(&bytes).map_err(beam_error)?;
        Ok(Self { keypair })
    }

    /// The public key other devices pin.
    #[wasm_bindgen(getter, js_name = publicKey)]
    #[must_use]
    pub fn public_key(&self) -> Vec<u8> {
        self.keypair.public().as_bytes().to_vec()
    }

    /// The private key, for saving. Never send it anywhere.
    #[wasm_bindgen(js_name = privateKey)]
    #[must_use]
    pub fn private_key(&self) -> Vec<u8> {
        self.keypair.private_bytes().to_vec()
    }

    /// The short, stable name of this identity, as the phone lists it.
    #[wasm_bindgen(getter, js_name = deviceId)]
    #[must_use]
    pub fn device_id(&self) -> String {
        self.keypair.public().device_id().to_string()
    }
}

/// The pairing code this browser shows for the phone to scan.
#[wasm_bindgen]
pub struct Invite {
    pub(crate) invite: PairingInvite,
}

#[wasm_bindgen]
impl Invite {
    /// A new invite with a fresh one-time token. Browsers can't accept connections, so it
    /// carries no addresses: the phone answers through the signaling room instead.
    ///
    /// # Errors
    ///
    /// If the cipher name is unknown or the random number generator fails.
    #[wasm_bindgen(constructor)]
    pub fn new(identity: &Identity, cipher_name: &str) -> Result<Self, JsValue> {
        let invite =
            PairingInvite::new(identity.keypair.public(), cipher(cipher_name)?, Vec::new())
                .map_err(beam_error)?;
        Ok(Self { invite })
    }

    /// The text to put in the QR code.
    #[must_use]
    pub fn text(&self) -> String {
        self.invite.encode()
    }

    /// The signaling room both devices join, derived from the token.
    #[wasm_bindgen(getter)]
    #[must_use]
    pub fn room(&self) -> Vec<u8> {
        self.invite.room().to_vec()
    }
}

/// A handshake waiting for a stream. The browser always opens the connection, so it always
/// sends the first handshake message.
#[wasm_bindgen]
pub struct BeamHandshake {
    handshake: Handshake,
}

#[wasm_bindgen]
impl BeamHandshake {
    /// Pairs with the phone that scanned `invite`.
    ///
    /// # Errors
    ///
    /// If the handshake can't be set up.
    pub fn pairing(identity: &Identity, invite: &Invite) -> Result<Self, JsValue> {
        let handshake = Handshake::pairing_displayer(
            &identity.keypair,
            invite.invite.cipher,
            Role::Initiator,
            &invite.invite.token,
        )
        .map_err(beam_error)?;
        Ok(Self { handshake })
    }

    /// Reconnects to a phone paired earlier, whose public key is `peer_key`.
    ///
    /// # Errors
    ///
    /// If the cipher name is unknown, `peer_key` isn't 32 bytes, or the handshake can't be set
    /// up.
    #[wasm_bindgen(js_name = knownDevice)]
    pub fn known_device(
        identity: &Identity,
        cipher_name: &str,
        peer_key: &[u8],
    ) -> Result<Self, JsValue> {
        let bytes: [u8; 32] = peer_key
            .try_into()
            .map_err(|_| invalid("a public key is 32 bytes"))?;
        let peer = PublicKey::from_bytes(bytes);
        let handshake = Handshake::known_initiator(&identity.keypair, cipher(cipher_name)?, peer)
            .map_err(beam_error)?;
        Ok(Self { handshake })
    }
}

/// A frame received from the phone.
#[wasm_bindgen]
pub struct ReceivedFrame {
    frame_type: u8,
    payload: Vec<u8>,
}

#[wasm_bindgen]
impl ReceivedFrame {
    /// The frame's type code (see the protocol's frame types).
    #[wasm_bindgen(getter, js_name = frameType)]
    #[must_use]
    #[allow(clippy::missing_const_for_fn)] // wasm_bindgen exports can't be const
    pub fn frame_type(&self) -> u8 {
        self.frame_type
    }

    /// The frame's body.
    #[wasm_bindgen(getter)]
    #[must_use]
    pub fn payload(&self) -> Vec<u8> {
        self.payload.clone()
    }
}

/// An authenticated, encrypted connection to the phone.
#[wasm_bindgen]
pub struct BeamConnection {
    pub(crate) connection: Connection<JsStream>,
}

#[wasm_bindgen]
impl BeamConnection {
    /// Runs `handshake` over a WebTransport bidirectional stream's two halves.
    ///
    /// # Errors
    ///
    /// A network error if the stream fails, a crypto error if the phone fails authentication.
    pub async fn establish(
        handshake: BeamHandshake,
        readable: ReadableStream,
        writable: WritableStream,
    ) -> Result<Self, JsValue> {
        let stream = JsStream::new(&readable, &writable)?;
        let connection = Connection::establish(stream, handshake.handshake)
            .await
            .map_err(beam_error)?;
        Ok(Self { connection })
    }

    /// The code both people compare to confirm a pairing, like `042 917`.
    #[wasm_bindgen(getter, js_name = verificationCode)]
    #[must_use]
    pub fn verification_code(&self) -> String {
        self.connection.verification_code().to_string()
    }

    /// The phone's public key, to remember it as trusted.
    #[wasm_bindgen(getter, js_name = peerKey)]
    #[must_use]
    pub fn peer_key(&self) -> Vec<u8> {
        self.connection.peer().as_bytes().to_vec()
    }

    /// Sends one frame on the control stream. Low level: transfers build on this.
    ///
    /// # Errors
    ///
    /// If the type code is unknown, the payload is too large, or the connection fails.
    pub async fn send(&mut self, frame_type: u8, payload: Vec<u8>) -> Result<(), JsValue> {
        let frame_type = FrameType::try_from(frame_type).map_err(beam_error)?;
        let frame = Frame::new(frame_type, CONTROL_STREAM, payload).map_err(beam_error)?;
        self.connection.send(&frame).await.map_err(beam_error)
    }

    /// The next frame, or `undefined` once the phone closed the connection.
    ///
    /// # Errors
    ///
    /// If the connection fails or the phone sends something invalid.
    pub async fn recv(&mut self) -> Result<Option<ReceivedFrame>, JsValue> {
        let frame = self.connection.recv().await.map_err(beam_error)?;
        Ok(frame.map(|frame| ReceivedFrame {
            frame_type: frame.frame_type as u8,
            payload: frame.payload,
        }))
    }

    /// Closes the sending side; the phone sees the end of the stream.
    ///
    /// # Errors
    ///
    /// If the stream fails while closing.
    pub async fn close(&mut self) -> Result<(), JsValue> {
        self.connection.close().await.map_err(beam_error)
    }
}
