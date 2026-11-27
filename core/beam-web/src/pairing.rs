//! What pairing needs around the handshake: opening the phone's offer from the signaling room
//! (where to connect, and the certificate to pin), and swapping names once connected (`HELLO`).

use beam_core::{DiscoveryKey, Hello, SignalFrom, SignalMessage, SignalSeal, exchange_hello};
use wasm_bindgen::prelude::*;

use crate::errors::{beam_error, invalid};
use crate::{BeamConnection, Identity, Invite};

/// Where the phone that scanned the code accepts this browser's WebTransport session.
#[wasm_bindgen]
pub struct PhoneOffer {
    addresses: Vec<String>,
    certificate_hash: Vec<u8>,
}

#[wasm_bindgen]
impl PhoneOffer {
    /// The phone's addresses with ports, like `192.168.1.20:47123` or `[2001:db8::7]:47123`,
    /// ready for `https://${address}/beam`. Try them all and keep the first that pairs.
    #[wasm_bindgen(getter)]
    #[must_use]
    pub fn addresses(&self) -> Vec<String> {
        self.addresses.clone()
    }

    /// The SHA-256 of the phone's certificate, for WebTransport's `serverCertificateHashes`.
    #[wasm_bindgen(getter, js_name = certificateHash)]
    #[must_use]
    pub fn certificate_hash(&self) -> Vec<u8> {
        self.certificate_hash.clone()
    }
}

#[wasm_bindgen]
impl Invite {
    /// Opens a message the phone posted in this invite's signaling room.
    ///
    /// # Errors
    ///
    /// `crypto` if it wasn't sealed by the phone that scanned this invite (another pairing's
    /// message, or one the server changed), `invalid` if it holds no address.
    #[wasm_bindgen(js_name = openOffer)]
    pub fn open_offer(&self, sealed: &[u8]) -> Result<PhoneOffer, JsValue> {
        let seal = SignalSeal::for_invite(&self.invite).map_err(beam_error)?;
        open_offer(&seal, sealed)
    }
}

/// Opens an offer the phone sealed with `seal`.
pub(crate) fn open_offer(seal: &SignalSeal, sealed: &[u8]) -> Result<PhoneOffer, JsValue> {
    let SignalMessage::Offer {
        hints,
        certificate_hash,
    } = seal.open(SignalFrom::Scanner, sealed).map_err(beam_error)?;
    if hints.is_empty() {
        return Err(invalid("the phone's offer has no address"));
    }
    Ok(PhoneOffer {
        addresses: hints.iter().map(ToString::to_string).collect(),
        certificate_hash: certificate_hash.to_vec(),
    })
}

#[wasm_bindgen]
impl Identity {
    /// The key this browser sends when pairing, which phones use to recognise a device's
    /// beacons. Browsers don't announce any, but the protocol always carries one.
    #[wasm_bindgen(getter, js_name = discoveryKey)]
    #[must_use]
    pub fn discovery_key(&self) -> Vec<u8> {
        DiscoveryKey::derive(&self.keypair, 0).as_bytes().to_vec()
    }
}

/// The phone this browser just paired with. Store it to reconnect later.
#[wasm_bindgen(getter_with_clone)]
pub struct PairedPhone {
    /// The name the phone gave, for the list of paired devices.
    pub name: String,
    /// Its public key, which later connections must prove (`BeamHandshake.knownDevice`).
    #[wasm_bindgen(js_name = publicKey)]
    pub public_key: Vec<u8>,
    /// Its device ID.
    #[wasm_bindgen(js_name = deviceId)]
    pub device_id: String,
}

#[wasm_bindgen]
impl BeamConnection {
    /// Right after a pairing handshake, tells the phone this browser's `name` and learns the
    /// phone's. `discovery_key` is [`Identity::discovery_key`].
    ///
    /// # Errors
    ///
    /// `invalid` if `discovery_key` isn't 32 bytes, a protocol error if the phone doesn't
    /// answer with its own `HELLO` or runs an incompatible version, a network error if the
    /// connection fails.
    pub async fn introduce(
        &mut self,
        name: String,
        discovery_key: Vec<u8>,
    ) -> Result<PairedPhone, JsValue> {
        let key = DiscoveryKey::from_slice(&discovery_key)
            .map_err(|_| invalid("a discovery key is 32 bytes"))?;
        let peer = exchange_hello(&mut self.connection, &Hello::new(&name, key))
            .await
            .map_err(beam_error)?;
        let phone = self.connection.peer();
        Ok(PairedPhone {
            name: peer.name,
            public_key: phone.as_bytes().to_vec(),
            device_id: phone.device_id().to_string(),
        })
    }
}
