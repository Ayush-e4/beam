//! Short codes that let two people confirm they paired with each other.

use std::fmt;

const CONTEXT: &str = "beam 2026-11-24 verification code v1";

/// A six-digit code derived from the handshake transcript. Matching codes on both screens
/// prove that no one sits between the two devices.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct VerificationCode(u32);

impl VerificationCode {
    pub(crate) fn derive(handshake_hash: &[u8]) -> Self {
        let key = blake3::derive_key(CONTEXT, handshake_hash);
        let mut first = [0u8; 8];
        first.copy_from_slice(&key[..8]);
        let digits = u64::from_le_bytes(first) % 1_000_000;
        Self(u32::try_from(digits).unwrap_or_default())
    }

    /// The code as a number below one million.
    #[must_use]
    pub const fn value(self) -> u32 {
        self.0
    }
}

impl fmt::Display for VerificationCode {
    /// Shown as two groups of three digits, for example `042 917`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:03} {:03}", self.0 / 1000, self.0 % 1000)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shows_six_padded_digits() {
        assert_eq!(VerificationCode(42_917).to_string(), "042 917");
        assert_eq!(VerificationCode(7).to_string(), "000 007");
    }

    #[test]
    fn is_deterministic_and_below_one_million() {
        let code = VerificationCode::derive(b"transcript");
        assert!(code.value() < 1_000_000);
        assert_eq!(code, VerificationCode::derive(b"transcript"));
        assert_ne!(code, VerificationCode::derive(b"other transcript"));
    }
}
