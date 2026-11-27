//! Meeting a phone this browser paired with earlier, without a pairing code (see
//! `protocol.md`, Meeting again): both derive the same signaling room from their keys, the
//! phone posts sealed offers there while it waits, and the browser connects with
//! `BeamHandshake.knownDevice`.

use beam_core::{Meeting, PublicKey};
use wasm_bindgen::prelude::*;

use crate::Identity;
use crate::errors::{beam_error, invalid};
use crate::pairing::{PhoneOffer, open_offer};

/// Where this browser meets one trusted phone.
#[wasm_bindgen]
pub struct PhoneMeeting {
    meeting: Meeting,
}

#[wasm_bindgen]
impl PhoneMeeting {
    /// The signaling room to join (`/rooms/{room as hex}`).
    #[wasm_bindgen(getter)]
    #[must_use]
    pub fn room(&self) -> Vec<u8> {
        self.meeting.room().to_vec()
    }

    /// Opens an offer the phone posted in the room. The phone posts a fresh one each time
    /// this browser joins; the room may also hand over an older one first, so try each as it
    /// comes.
    ///
    /// # Errors
    ///
    /// `crypto` if that phone didn't seal it for this browser, `invalid` if it holds no
    /// address.
    #[wasm_bindgen(js_name = openOffer)]
    pub fn open_offer(&self, sealed: &[u8]) -> Result<PhoneOffer, JsValue> {
        open_offer(self.meeting.seal(), sealed)
    }
}

#[wasm_bindgen]
impl Identity {
    /// Where to meet the trusted phone whose public key is `phone_key` (from pairing).
    ///
    /// # Errors
    ///
    /// `invalid` if `phone_key` isn't 32 bytes, `crypto` if it isn't a usable key.
    pub fn meet(&self, phone_key: &[u8]) -> Result<PhoneMeeting, JsValue> {
        let bytes: [u8; 32] = phone_key
            .try_into()
            .map_err(|_| invalid("a public key is 32 bytes"))?;
        let meeting =
            Meeting::between(&self.keypair, &PublicKey::from_bytes(bytes)).map_err(beam_error)?;
        Ok(PhoneMeeting { meeting })
    }
}
