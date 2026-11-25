//! RFC 4648 base32 without padding. Its alphabet (A–Z, 2–7) fits QR codes' compact
//! alphanumeric mode, which keeps pairing codes small and easy to scan.

const ALPHABET: &[u8; 32] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";

/// Encodes `bytes` as unpadded upper-case base32.
pub(crate) fn encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(5) * 8);
    let (mut buffer, mut bits) = (0u32, 0u32);
    for &byte in bytes {
        buffer = (buffer << 8) | u32::from(byte);
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            out.push(char::from(ALPHABET[((buffer >> bits) & 31) as usize]));
        }
    }
    if bits > 0 {
        out.push(char::from(ALPHABET[((buffer << (5 - bits)) & 31) as usize]));
    }
    out
}

/// Decodes unpadded upper-case base32; `None` for any other character or leftover bits that
/// aren't zero (so every byte string has exactly one encoding).
pub(crate) fn decode(text: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(text.len() * 5 / 8);
    let (mut buffer, mut bits) = (0u32, 0u32);
    for c in text.bytes() {
        let value = ALPHABET.iter().position(|&a| a == c)?;
        buffer = (buffer << 5) | u32::try_from(value).ok()?;
        bits += 5;
        if bits >= 8 {
            bits -= 8;
            out.push(u8::try_from((buffer >> bits) & 0xFF).ok()?);
        }
        buffer &= (1 << bits) - 1;
    }
    (bits < 5 && buffer == 0).then_some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_the_rfc_vectors() {
        for (plain, encoded) in [
            ("", ""),
            ("f", "MY"),
            ("fo", "MZXQ"),
            ("foo", "MZXW6"),
            ("foob", "MZXW6YQ"),
            ("fooba", "MZXW6YTB"),
            ("foobar", "MZXW6YTBOI"),
        ] {
            assert_eq!(encode(plain.as_bytes()), encoded);
            assert_eq!(decode(encoded).unwrap(), plain.as_bytes());
        }
    }

    #[test]
    fn rejects_other_characters_and_non_canonical_tails() {
        assert!(decode("mzxw6").is_none(), "lower case");
        assert!(decode("MZXW6=").is_none(), "padding");
        assert!(decode("MZ1").is_none(), "outside the alphabet");
        assert!(decode("MZ").is_none(), "leftover bits must be zero");
        assert!(decode("M").is_none(), "too few bits for a byte");
    }

    #[test]
    fn round_trips_every_length() {
        let bytes: Vec<u8> = (0..=255).collect();
        for len in 0..bytes.len() {
            assert_eq!(decode(&encode(&bytes[..len])).unwrap(), &bytes[..len]);
        }
    }
}
