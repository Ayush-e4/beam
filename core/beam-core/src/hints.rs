//! Connection hints: the addresses and ports where a device accepts connections, as pairing
//! codes and signaling messages carry them.
//!
//! ```text
//! hint count u8 (≤ 4) | hint = family u8 (4 or 6) | address [4 or 16] | port u16 BE
//! ```

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};

use crate::{BeamError, Result};

/// Most connection hints a device gives.
pub const MAX_HINTS: usize = 4;

/// Refuses more than [`MAX_HINTS`] hints and hints without a port.
pub(crate) fn check(hints: &[SocketAddr]) -> Result<()> {
    if hints.len() > MAX_HINTS {
        return Err(malformed("too many connection hints"));
    }
    if hints.iter().any(|hint| hint.port() == 0) {
        return Err(malformed("connection hint without a port"));
    }
    Ok(())
}

/// Appends the first [`MAX_HINTS`] of `hints`, with their count, to `bytes`.
pub(crate) fn encode(hints: &[SocketAddr], bytes: &mut Vec<u8>) {
    let kept = &hints[..hints.len().min(MAX_HINTS)];
    // At most MAX_HINTS (4), so it fits.
    bytes.push(u8::try_from(kept.len()).unwrap_or(0));
    for hint in kept {
        match hint.ip() {
            IpAddr::V4(ip) => {
                bytes.push(4);
                bytes.extend_from_slice(&ip.octets());
            }
            IpAddr::V6(ip) => {
                bytes.push(6);
                bytes.extend_from_slice(&ip.octets());
            }
        }
        bytes.extend_from_slice(&hint.port().to_be_bytes());
    }
}

/// Reads hints from the start of `bytes`; returns them and what follows.
///
/// # Errors
///
/// [`BeamError::Malformed`] if they're truncated, too many, of an unknown family, or lack a port.
pub(crate) fn decode(bytes: &[u8]) -> Result<(Vec<SocketAddr>, &[u8])> {
    let (&count, mut rest) = bytes
        .split_first()
        .ok_or_else(|| malformed("no hint count"))?;
    let count = usize::from(count);
    if count > MAX_HINTS {
        return Err(malformed("too many connection hints"));
    }
    let mut hints = Vec::with_capacity(count);
    for _ in 0..count {
        let (&family, after) = rest.split_first().ok_or_else(truncated)?;
        let (ip, after) = match family {
            4 => {
                let (octets, after) = after.split_first_chunk::<4>().ok_or_else(truncated)?;
                (IpAddr::V4(Ipv4Addr::from(*octets)), after)
            }
            6 => {
                let (octets, after) = after.split_first_chunk::<16>().ok_or_else(truncated)?;
                (IpAddr::V6(Ipv6Addr::from(*octets)), after)
            }
            _ => return Err(malformed("unknown address family")),
        };
        let (port, after) = after.split_first_chunk::<2>().ok_or_else(truncated)?;
        hints.push(SocketAddr::new(ip, u16::from_be_bytes(*port)));
        rest = after;
    }
    check(&hints)?;
    Ok((hints, rest))
}

fn truncated() -> BeamError {
    malformed("truncated connection hint")
}

fn malformed(why: &str) -> BeamError {
    BeamError::Malformed(why.into())
}
