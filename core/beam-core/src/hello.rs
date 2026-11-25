//! `HELLO`: the first frame each side sends after a pairing handshake, carrying the protocol
//! version, the name people see in their trusted-device list and the key that lets the peer
//! recognise this device's discovery beacons.
//!
//! ```text
//! HELLO : major u16 | minor u16 | name_len u8 | name (UTF-8, at most 64 bytes)
//!         | discovery_key (32 bytes)
//! ```

use tokio::io::{AsyncRead, AsyncWrite};

use crate::discovery::{DISCOVERY_KEY_LEN, DiscoveryKey};
use crate::frame::{Frame, FrameType};
use crate::wire::{Reader, Writer};
use crate::{BeamError, Connection, ProtocolVersion, Result};

/// Longest device name, in bytes.
pub const MAX_NAME_LEN: usize = 64;

/// Stream for connection-level frames such as `HELLO`.
const CONTROL_STREAM: u32 = 0;

/// What a device says about itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hello {
    /// The protocol version it speaks.
    pub version: ProtocolVersion,
    /// Its display name, such as "Ayush's Galaxy S21".
    pub name: String,
    /// The key its discovery beacons are made with. Secret: only sent once encrypted.
    pub discovery_key: DiscoveryKey,
}

impl Hello {
    /// This build's hello with `name`, trimmed, without control characters and cut to
    /// [`MAX_NAME_LEN`] bytes on a character boundary, and this device's `discovery_key`.
    #[must_use]
    pub fn new(name: &str, discovery_key: DiscoveryKey) -> Self {
        let mut clean = String::new();
        for c in name.trim().chars().filter(|c| !c.is_control()) {
            if clean.len() + c.len_utf8() > MAX_NAME_LEN {
                break;
            }
            clean.push(c);
        }
        Self {
            version: ProtocolVersion::CURRENT,
            name: clean,
            discovery_key,
        }
    }

    /// The `HELLO` payload.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let name = &self.name.as_bytes()[..self.name.len().min(MAX_NAME_LEN)];
        let mut writer = Writer::new();
        writer
            .u16(self.version.major)
            .u16(self.version.minor)
            .u8(u8::try_from(name.len()).unwrap_or(0))
            .bytes(name)
            .bytes(self.discovery_key.as_bytes());
        writer.finish()
    }

    /// Parses a `HELLO` payload.
    ///
    /// # Errors
    ///
    /// [`BeamError::Malformed`] if it is truncated, too long, has trailing bytes or a name
    /// that isn't UTF-8 or contains control characters.
    pub fn decode(payload: &[u8]) -> Result<Self> {
        let mut reader = Reader::new(payload);
        let version = ProtocolVersion {
            major: reader.u16()?,
            minor: reader.u16()?,
        };
        let len = usize::from(reader.u8()?);
        if len > MAX_NAME_LEN {
            return Err(BeamError::Malformed("device name is too long".into()));
        }
        let name = reader.utf8(len)?;
        if name.chars().any(char::is_control) {
            return Err(BeamError::Malformed(
                "device name has control characters".into(),
            ));
        }
        let discovery_key = DiscoveryKey::from_slice(reader.bytes(DISCOVERY_KEY_LEN)?)?;
        reader.finish()?;
        Ok(Self {
            version,
            name: name.to_owned(),
            discovery_key,
        })
    }
}

/// Sends `local` and returns the peer's hello.
///
/// # Errors
///
/// Network errors if the link fails, [`BeamError::Malformed`] if the peer's first frame isn't
/// a valid `HELLO`, and [`BeamError::IncompatibleVersion`] for a different major version.
pub async fn exchange_hello<S: AsyncRead + AsyncWrite + Unpin>(
    conn: &mut Connection<S>,
    local: &Hello,
) -> Result<Hello> {
    conn.send(&Frame::new(
        FrameType::Hello,
        CONTROL_STREAM,
        local.encode(),
    )?)
    .await?;
    let frame = conn
        .recv()
        .await?
        .ok_or_else(|| BeamError::Malformed("the peer closed before saying hello".into()))?;
    if frame.frame_type != FrameType::Hello {
        return Err(BeamError::Malformed(format!(
            "expected HELLO, got {:?}",
            frame.frame_type
        )));
    }
    let peer = Hello::decode(&frame.payload)?;
    local.version.ensure_compatible(peer.version)?;
    Ok(peer)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key() -> DiscoveryKey {
        DiscoveryKey::from_slice(&[9; DISCOVERY_KEY_LEN]).unwrap()
    }

    #[test]
    fn round_trips() {
        let hello = Hello::new("Ayush's Galaxy S21 FE", key());
        let decoded = Hello::decode(&hello.encode()).unwrap();
        assert_eq!(decoded, hello);
        assert_eq!(decoded.discovery_key, key());
    }

    #[test]
    fn names_are_cleaned_and_cut_on_a_character_boundary() {
        assert_eq!(Hello::new("  Pixel\n9\u{7}  ", key()).name, "Pixel9");
        let long = "é".repeat(40); // 80 bytes
        let cut = Hello::new(&long, key()).name;
        assert_eq!(cut.len(), MAX_NAME_LEN);
        assert!(cut.chars().all(|c| c == 'é'));
    }

    #[test]
    fn bad_payloads_are_rejected() {
        let with_key = |mut head: Vec<u8>| {
            head.extend_from_slice(&[0; DISCOVERY_KEY_LEN]);
            head
        };
        let mut long = vec![1, 0, 0, 0, 65];
        long.extend_from_slice(&[b'a'; 65]);
        let mut trailing = Hello::new("a", key()).encode();
        trailing.push(0);
        let mut short_key = Hello::new("a", key()).encode();
        short_key.pop();
        for payload in [
            vec![1, 0, 0],                             // truncated
            with_key(long),                            // name too long
            with_key(vec![1, 0, 0, 0, 2, 0xC3, 0x28]), // not UTF-8
            with_key(vec![1, 0, 0, 0, 1, b'\n']),      // control character
            short_key,
            trailing,
        ] {
            assert!(Hello::decode(&payload).is_err(), "{payload:?}");
        }
    }
}
