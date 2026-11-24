//! Little-endian reading and writing for message payloads.
//!
//! Every read is bounds-checked and returns [`BeamError::Malformed`] instead of panicking, so
//! decoders built on [`Reader`] are safe to run on hostile input.

use crate::{BeamError, Result};

/// Appends little-endian values to a buffer.
#[derive(Debug, Default)]
pub(crate) struct Writer {
    buf: Vec<u8>,
}

impl Writer {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn u8(&mut self, value: u8) -> &mut Self {
        self.buf.push(value);
        self
    }

    pub(crate) fn u16(&mut self, value: u16) -> &mut Self {
        self.bytes(&value.to_le_bytes())
    }

    pub(crate) fn u32(&mut self, value: u32) -> &mut Self {
        self.bytes(&value.to_le_bytes())
    }

    pub(crate) fn u64(&mut self, value: u64) -> &mut Self {
        self.bytes(&value.to_le_bytes())
    }

    pub(crate) fn i64(&mut self, value: i64) -> &mut Self {
        self.bytes(&value.to_le_bytes())
    }

    pub(crate) fn bytes(&mut self, value: &[u8]) -> &mut Self {
        self.buf.extend_from_slice(value);
        self
    }

    /// Writes a count as `u32`, failing if it does not fit.
    pub(crate) fn count(&mut self, count: usize) -> Result<&mut Self> {
        let count = u32::try_from(count)
            .map_err(|_| BeamError::Malformed("too many entries to encode".into()))?;
        Ok(self.u32(count))
    }

    pub(crate) fn finish(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.buf)
    }
}

/// Reads little-endian values from a payload.
#[derive(Debug)]
pub(crate) struct Reader<'a> {
    rest: &'a [u8],
}

impl<'a> Reader<'a> {
    pub(crate) fn new(payload: &'a [u8]) -> Self {
        Self { rest: payload }
    }

    pub(crate) fn bytes(&mut self, len: usize) -> Result<&'a [u8]> {
        if len > self.rest.len() {
            return Err(BeamError::Malformed("message is truncated".into()));
        }
        let (head, tail) = self.rest.split_at(len);
        self.rest = tail;
        Ok(head)
    }

    pub(crate) fn array<const N: usize>(&mut self) -> Result<[u8; N]> {
        let mut out = [0u8; N];
        out.copy_from_slice(self.bytes(N)?);
        Ok(out)
    }

    pub(crate) fn u8(&mut self) -> Result<u8> {
        Ok(self.array::<1>()?[0])
    }

    pub(crate) fn u16(&mut self) -> Result<u16> {
        Ok(u16::from_le_bytes(self.array()?))
    }

    pub(crate) fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(self.array()?))
    }

    pub(crate) fn u64(&mut self) -> Result<u64> {
        Ok(u64::from_le_bytes(self.array()?))
    }

    pub(crate) fn i64(&mut self) -> Result<i64> {
        Ok(i64::from_le_bytes(self.array()?))
    }

    pub(crate) fn bool(&mut self) -> Result<bool> {
        match self.u8()? {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(BeamError::Malformed("invalid flag byte".into())),
        }
    }

    /// Reads a `u32` count and checks that `count` entries of at least `min_entry_len` bytes
    /// can fit in what is left, so a forged count cannot trigger a huge allocation.
    pub(crate) fn count(&mut self, min_entry_len: usize) -> Result<usize> {
        let count = usize::try_from(self.u32()?)
            .map_err(|_| BeamError::Malformed("count does not fit in memory".into()))?;
        if count.saturating_mul(min_entry_len.max(1)) > self.rest.len() {
            return Err(BeamError::Malformed(
                "count exceeds the message size".into(),
            ));
        }
        Ok(count)
    }

    pub(crate) fn utf8(&mut self, len: usize) -> Result<&'a str> {
        std::str::from_utf8(self.bytes(len)?)
            .map_err(|_| BeamError::Malformed("text is not valid UTF-8".into()))
    }

    /// Everything not read yet.
    pub(crate) fn rest(&mut self) -> &'a [u8] {
        std::mem::take(&mut self.rest)
    }

    /// Fails if unread bytes remain, so trailing garbage is never silently accepted.
    pub(crate) fn finish(&self) -> Result<()> {
        if self.rest.is_empty() {
            Ok(())
        } else {
            Err(BeamError::Malformed("message has trailing bytes".into()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_round_trip() {
        let payload = Writer::new()
            .u8(7)
            .u16(0xBEEF)
            .u32(1)
            .u64(u64::MAX)
            .i64(-5)
            .bytes(b"hi")
            .finish();
        let mut r = Reader::new(&payload);
        assert_eq!(r.u8().unwrap(), 7);
        assert_eq!(r.u16().unwrap(), 0xBEEF);
        assert_eq!(r.u32().unwrap(), 1);
        assert_eq!(r.u64().unwrap(), u64::MAX);
        assert_eq!(r.i64().unwrap(), -5);
        assert_eq!(r.utf8(2).unwrap(), "hi");
        r.finish().unwrap();
    }

    #[test]
    fn truncation_trailing_bytes_and_bad_flags_are_errors() {
        assert!(Reader::new(&[1, 2]).u32().is_err());
        assert!(Reader::new(&[1]).finish().is_err());
        assert!(Reader::new(&[2]).bool().is_err());
        assert!(Reader::new(&[0xFF, 0xFE]).utf8(2).is_err());
    }

    #[test]
    fn forged_counts_cannot_force_huge_allocations() {
        let payload = Writer::new().u32(u32::MAX).bytes(&[0; 10]).finish();
        assert!(Reader::new(&payload).count(4).is_err());
        let payload = Writer::new().u32(2).bytes(&[0; 8]).finish();
        assert_eq!(Reader::new(&payload).count(4).unwrap(), 2);
    }
}
