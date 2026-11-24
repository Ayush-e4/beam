//! Wire framing for the Beam session layer.
//!
//! Every message is a frame with a fixed 9-byte header followed by its payload:
//!
//! ```text
//! +--------+---------------+---------------+------------------+
//! | type   | stream_id     | length        | payload          |
//! | u8     | u32 (BE)      | u32 (BE)      | `length` bytes   |
//! +--------+---------------+---------------+------------------+
//! ```
//!
//! Decoding is incremental: callers feed whatever bytes have arrived and get back either a
//! complete frame or a request for more data. Payload sizes are capped so a malicious peer cannot
//! make us allocate unbounded memory.

use crate::{BeamError, Result};

/// Size of the frame header in bytes.
pub const HEADER_LEN: usize = 9;

/// Largest payload accepted in a single frame (16 MiB).
pub const MAX_PAYLOAD_LEN: usize = 16 * 1024 * 1024;

/// Kinds of frames defined by protocol version 1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum FrameType {
    /// Protocol version and capabilities.
    Hello = 0x01,
    /// Transfer manifest proposed by the sender.
    Offer = 0x02,
    /// Receiver accepts an offer.
    Accept = 0x03,
    /// Receiver rejects an offer.
    Reject = 0x04,
    /// File data.
    Chunk = 0x05,
    /// Verified chunk ranges.
    Ack = 0x06,
    /// Verified ranges for an interrupted transfer.
    Resume = 0x07,
    /// Clipboard item.
    Clipboard = 0x08,
    /// Abort a transfer.
    Cancel = 0x09,
    /// Liveness probe.
    Ping = 0x0A,
    /// Liveness reply.
    Pong = 0x0B,
    /// Typed error.
    Error = 0x0C,
}

impl TryFrom<u8> for FrameType {
    type Error = BeamError;

    fn try_from(value: u8) -> Result<Self> {
        Ok(match value {
            0x01 => Self::Hello,
            0x02 => Self::Offer,
            0x03 => Self::Accept,
            0x04 => Self::Reject,
            0x05 => Self::Chunk,
            0x06 => Self::Ack,
            0x07 => Self::Resume,
            0x08 => Self::Clipboard,
            0x09 => Self::Cancel,
            0x0A => Self::Ping,
            0x0B => Self::Pong,
            0x0C => Self::Error,
            other => {
                return Err(BeamError::Malformed(format!(
                    "unknown frame type 0x{other:02x}"
                )));
            }
        })
    }
}

/// A single protocol frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    /// What the frame carries.
    pub frame_type: FrameType,
    /// Logical stream the frame belongs to (0 is the control stream).
    pub stream_id: u32,
    /// Frame body.
    pub payload: Vec<u8>,
}

/// Outcome of trying to decode a frame from a buffer.
#[derive(Debug, PartialEq, Eq)]
pub enum Decoded {
    /// A full frame was decoded; `consumed` bytes can be dropped from the buffer.
    Frame {
        /// The decoded frame.
        frame: Frame,
        /// Number of bytes the frame occupied.
        consumed: usize,
    },
    /// The buffer does not yet hold a complete frame; at least this many more bytes are needed.
    NeedMore(usize),
}

impl Frame {
    /// Creates a frame, rejecting payloads above [`MAX_PAYLOAD_LEN`].
    ///
    /// # Errors
    ///
    /// Returns [`BeamError::Malformed`] if the payload is too large.
    pub fn new(frame_type: FrameType, stream_id: u32, payload: Vec<u8>) -> Result<Self> {
        check_len(payload.len())?;
        Ok(Self {
            frame_type,
            stream_id,
            payload,
        })
    }

    /// Appends the encoded frame to `out`.
    ///
    /// # Errors
    ///
    /// Returns [`BeamError::Malformed`] if the payload exceeds [`MAX_PAYLOAD_LEN`].
    pub fn encode_into(&self, out: &mut Vec<u8>) -> Result<()> {
        let len = check_len(self.payload.len())?;
        out.reserve(HEADER_LEN + self.payload.len());
        out.push(self.frame_type as u8);
        out.extend_from_slice(&self.stream_id.to_be_bytes());
        out.extend_from_slice(&len.to_be_bytes());
        out.extend_from_slice(&self.payload);
        Ok(())
    }

    /// Decodes one frame from the start of `buf`.
    ///
    /// # Errors
    ///
    /// Returns [`BeamError::Malformed`] for an unknown frame type or an oversized payload.
    pub fn decode(buf: &[u8]) -> Result<Decoded> {
        let Some(header) = buf.get(..HEADER_LEN) else {
            return Ok(Decoded::NeedMore(HEADER_LEN - buf.len()));
        };
        let frame_type = FrameType::try_from(header[0])?;
        let stream_id = read_u32(&header[1..5]);
        let len = usize::try_from(read_u32(&header[5..9]))
            .map_err(|_| BeamError::Malformed("frame length does not fit in memory".into()))?;
        check_len(len)?;

        let total = HEADER_LEN + len;
        let Some(payload) = buf.get(HEADER_LEN..total) else {
            return Ok(Decoded::NeedMore(total - buf.len()));
        };
        let frame = Frame {
            frame_type,
            stream_id,
            payload: payload.to_vec(),
        };
        Ok(Decoded::Frame {
            frame,
            consumed: total,
        })
    }
}

/// Validates a payload length and converts it to the on-wire `u32`.
fn check_len(len: usize) -> Result<u32> {
    if len > MAX_PAYLOAD_LEN {
        return Err(BeamError::Malformed(format!(
            "payload of {len} bytes exceeds the {MAX_PAYLOAD_LEN}-byte limit"
        )));
    }
    u32::try_from(len).map_err(|_| BeamError::Malformed("payload length overflows u32".into()))
}

/// Reads a big-endian `u32` from a 4-byte slice taken from a validated header.
fn read_u32(bytes: &[u8]) -> u32 {
    let mut array = [0u8; 4];
    array.copy_from_slice(bytes);
    u32::from_be_bytes(array)
}

#[cfg(test)]
#[path = "frame_tests.rs"]
mod tests;
