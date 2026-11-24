//! Encrypted session layer over a reliable, ordered byte stream (a TCP connection or a QUIC
//! stream).
//!
//! Frames are encoded into a plaintext stream, which is cut into encrypted records:
//!
//! ```text
//! record : length u16 BE (2) | ciphertext (`length` bytes, at most 65 535, includes 16-byte tag)
//! ```
//!
//! Nonces are implicit counters, so a record that is dropped, replayed, reordered or altered
//! fails authentication. Any failure breaks the session for good; the caller must reconnect
//! (transfers then continue from the resume journal). Both directions switch to fresh keys every
//! [`REKEY_INTERVAL`] records, at the same point on each side, so no key is used for too long.
//!
//! Like the handshake, the session does no I/O: [`Session::seal`] appends bytes to send and
//! [`Session::receive`] takes whatever bytes arrived and returns the complete frames.

use snow::TransportState;

use crate::frame::{Decoded, Frame};
use crate::handshake::Established;
use crate::keys::PublicKey;
use crate::{BeamError, Result};

/// Largest record on the wire, as fixed by Noise.
pub const MAX_RECORD_LEN: usize = 65_535;
/// Length of the authentication tag added to every record.
pub const TAG_LEN: usize = 16;
/// Largest amount of plaintext carried by one record.
pub const MAX_RECORD_PLAINTEXT: usize = MAX_RECORD_LEN - TAG_LEN;
/// Records sent in one direction before both sides move to new keys.
pub const REKEY_INTERVAL: u64 = 1 << 20;

pub(crate) const LEN_PREFIX: usize = 2;

/// An authenticated, encrypted session with one peer.
pub struct Session {
    peer: PublicKey,
    transport: TransportState,
    sent: u64,
    received: u64,
    rekey_interval: u64,
    /// Bytes of a record that has not fully arrived yet.
    pending_record: Vec<u8>,
    /// Decrypted bytes not yet decoded into frames; starts at `plain_start`.
    plain: Vec<u8>,
    plain_start: usize,
    broken: bool,
}

impl Session {
    /// Starts a session from a completed handshake.
    #[must_use]
    pub fn new(established: Established) -> Self {
        let peer = established.peer();
        Self {
            peer,
            transport: established.into_transport(),
            sent: 0,
            received: 0,
            rekey_interval: REKEY_INTERVAL,
            pending_record: Vec::with_capacity(LEN_PREFIX + MAX_RECORD_LEN),
            plain: Vec::new(),
            plain_start: 0,
            broken: false,
        }
    }

    /// Rekeys more often so tests can cross several key changes quickly.
    #[cfg(test)]
    fn with_rekey_interval(mut self, records: u64) -> Self {
        self.rekey_interval = records;
        self
    }

    /// The authenticated peer.
    #[must_use]
    pub const fn peer(&self) -> PublicKey {
        self.peer
    }

    /// Whether part of a record or frame has arrived without its remainder. A connection that
    /// closes in this state was cut off mid-message.
    #[must_use]
    pub fn has_partial_input(&self) -> bool {
        !self.pending_record.is_empty() || self.plain.len() > self.plain_start
    }

    /// Encrypts `frame` and appends the resulting records to `out`.
    ///
    /// # Errors
    ///
    /// Returns [`BeamError::Malformed`] for an oversized frame and [`BeamError::SessionBroken`]
    /// if the session already failed.
    pub fn seal(&mut self, frame: &Frame, out: &mut Vec<u8>) -> Result<()> {
        self.ensure_usable()?;
        let mut plain = Vec::new();
        frame.encode_into(&mut plain)?;
        for piece in plain.chunks(MAX_RECORD_PLAINTEXT) {
            if let Err(err) = self.seal_record(piece, out) {
                self.broken = true;
                return Err(err);
            }
        }
        Ok(())
    }

    /// Decrypts newly arrived bytes and returns every frame they complete.
    ///
    /// # Errors
    ///
    /// Returns [`BeamError::RecordRejected`] if a record fails authentication,
    /// [`BeamError::Malformed`] if the decrypted stream is not valid frames, and
    /// [`BeamError::SessionBroken`] if the session already failed.
    pub fn receive(&mut self, bytes: &[u8]) -> Result<Vec<Frame>> {
        self.ensure_usable()?;
        let result = self.receive_inner(bytes);
        if result.is_err() {
            self.broken = true;
        }
        result
    }

    fn receive_inner(&mut self, mut bytes: &[u8]) -> Result<Vec<Frame>> {
        let mut frames = Vec::new();
        while !bytes.is_empty() {
            let want = self.record_bytes_wanted();
            let take = want.min(bytes.len());
            self.pending_record.extend_from_slice(&bytes[..take]);
            bytes = &bytes[take..];
            if self.record_is_complete() {
                self.open_record()?;
                self.drain_frames(&mut frames)?;
            }
        }
        Ok(frames)
    }

    /// How many more bytes the pending record needs before it is complete.
    fn record_bytes_wanted(&self) -> usize {
        match self.pending_record.get(..LEN_PREFIX) {
            None => LEN_PREFIX - self.pending_record.len(),
            Some(prefix) => {
                let len = usize::from(u16::from_be_bytes([prefix[0], prefix[1]]));
                LEN_PREFIX + len - self.pending_record.len()
            }
        }
    }

    fn record_is_complete(&self) -> bool {
        self.pending_record.len() >= LEN_PREFIX && self.record_bytes_wanted() == 0
    }

    fn seal_record(&mut self, piece: &[u8], out: &mut Vec<u8>) -> Result<()> {
        let start = out.len();
        out.resize(start + LEN_PREFIX + piece.len() + TAG_LEN, 0);
        let len = self
            .transport
            .write_message(piece, &mut out[start + LEN_PREFIX..])
            .map_err(|_| BeamError::SessionBroken)?;
        let prefix = u16::try_from(len)
            .map_err(|_| BeamError::Malformed("record longer than 65 535 bytes".into()))?;
        out[start..start + LEN_PREFIX].copy_from_slice(&prefix.to_be_bytes());
        out.truncate(start + LEN_PREFIX + len);
        self.sent += 1;
        if self.sent.is_multiple_of(self.rekey_interval) {
            self.transport.rekey_outgoing();
        }
        Ok(())
    }

    fn open_record(&mut self) -> Result<()> {
        let ciphertext = &self.pending_record[LEN_PREFIX..];
        if ciphertext.len() <= TAG_LEN {
            return Err(BeamError::RecordRejected);
        }
        let start = self.plain.len();
        self.plain.resize(start + ciphertext.len(), 0);
        let len = self
            .transport
            .read_message(ciphertext, &mut self.plain[start..])
            .map_err(|_| BeamError::RecordRejected)?;
        self.plain.truncate(start + len);
        self.pending_record.clear();
        self.received += 1;
        if self.received.is_multiple_of(self.rekey_interval) {
            self.transport.rekey_incoming();
        }
        Ok(())
    }

    fn drain_frames(&mut self, frames: &mut Vec<Frame>) -> Result<()> {
        while let Decoded::Frame { frame, consumed } =
            Frame::decode(&self.plain[self.plain_start..])?
        {
            frames.push(frame);
            self.plain_start += consumed;
        }
        self.plain.drain(..self.plain_start);
        self.plain_start = 0;
        Ok(())
    }

    fn ensure_usable(&self) -> Result<()> {
        if self.broken {
            Err(BeamError::SessionBroken)
        } else {
            Ok(())
        }
    }
}

#[cfg(test)]
#[path = "session_tests.rs"]
mod tests;
