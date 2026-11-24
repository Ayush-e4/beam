//! Adaptive per-chunk zstd compression.
//!
//! Compression pays off only for compressible data on links slower than the CPU, so it is
//! decided in two steps:
//!
//! 1. [`worth_compressing`] runs once per file: formats that are already compressed (JPEG, MP4,
//!    ZIP, …) are skipped by MIME type, and anything else is skipped unless a sample of it
//!    shrinks by at least [`MIN_SAVING_PERCENT`].
//! 2. [`encode_chunk`] runs per chunk and still falls back to raw bytes whenever compression
//!    would not save enough, so a mixed file never grows.
//!
//! Chunk hashes always cover the original bytes; the receiver decompresses first, then
//! verifies. [`decode_chunk`] never produces more than the chunk's declared size, so a
//! malicious peer cannot send a decompression bomb.

use crate::{BeamError, Result};

/// zstd level: fast enough to keep up with Wi-Fi speeds on one phone core.
pub const LEVEL: i32 = 1;
/// Minimum size reduction, in percent, for compression to be used.
pub const MIN_SAVING_PERCENT: usize = 10;
/// How many leading bytes of a file are sampled to decide.
pub const SAMPLE_LEN: usize = 64 * 1024;

/// How a chunk's bytes are encoded on the wire.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum ChunkEncoding {
    /// The original bytes.
    Raw = 0,
    /// zstd-compressed bytes.
    Zstd = 1,
}

impl TryFrom<u8> for ChunkEncoding {
    type Error = BeamError;

    fn try_from(value: u8) -> Result<Self> {
        match value {
            0 => Ok(Self::Raw),
            1 => Ok(Self::Zstd),
            other => Err(BeamError::Malformed(format!(
                "unknown chunk encoding {other}"
            ))),
        }
    }
}

/// A chunk ready to send.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EncodedChunk {
    /// How `bytes` is encoded.
    pub encoding: ChunkEncoding,
    /// The bytes to put on the wire.
    pub bytes: Vec<u8>,
}

/// Decides whether a file is worth compressing, from its MIME type and its first bytes.
#[must_use]
pub fn worth_compressing(mime: &str, sample: &[u8]) -> bool {
    if is_precompressed(mime) || sample.is_empty() {
        return false;
    }
    let sample = &sample[..sample.len().min(SAMPLE_LEN)];
    zstd::bulk::compress(sample, LEVEL).is_ok_and(|packed| saves_enough(sample.len(), packed.len()))
}

/// Encodes one chunk, compressing only if `compress` is set and it saves enough.
///
/// # Errors
///
/// Returns a storage error if the compressor fails to initialise.
pub fn encode_chunk(data: &[u8], compress: bool) -> Result<EncodedChunk> {
    if compress && !data.is_empty() {
        let packed = zstd::bulk::compress(data, LEVEL).map_err(BeamError::storage)?;
        if saves_enough(data.len(), packed.len()) {
            return Ok(EncodedChunk {
                encoding: ChunkEncoding::Zstd,
                bytes: packed,
            });
        }
    }
    Ok(EncodedChunk {
        encoding: ChunkEncoding::Raw,
        bytes: data.to_vec(),
    })
}

/// Decodes a received chunk whose original size must be exactly `expected_len`.
///
/// # Errors
///
/// Returns [`BeamError::Malformed`] if the data is not valid zstd or does not decode to
/// exactly `expected_len` bytes.
pub fn decode_chunk(encoding: ChunkEncoding, bytes: &[u8], expected_len: usize) -> Result<Vec<u8>> {
    let data = match encoding {
        ChunkEncoding::Raw => bytes.to_vec(),
        ChunkEncoding::Zstd => zstd::bulk::decompress(bytes, expected_len)
            .map_err(|err| BeamError::Malformed(format!("bad compressed chunk: {err}")))?,
    };
    if data.len() == expected_len {
        Ok(data)
    } else {
        Err(BeamError::Malformed(format!(
            "chunk decoded to {} bytes, expected {expected_len}",
            data.len()
        )))
    }
}

fn saves_enough(original: usize, packed: usize) -> bool {
    packed.saturating_mul(100) <= original.saturating_mul(100 - MIN_SAVING_PERCENT)
}

/// Formats that are already compressed, where zstd only burns CPU.
fn is_precompressed(mime: &str) -> bool {
    let mime = mime
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    let (kind, subtype) = mime.split_once('/').unwrap_or((mime.as_str(), ""));
    match kind {
        "video" => true,
        "image" => !matches!(subtype, "bmp" | "svg+xml" | "x-portable-pixmap" | "tiff"),
        "audio" => !matches!(subtype, "wav" | "x-wav" | "wave" | "aiff" | "x-aiff"),
        "font" => matches!(subtype, "woff" | "woff2"),
        "application" => {
            subtype.contains("zip")
                || subtype.starts_with("vnd.openxmlformats")
                || subtype.starts_with("vnd.oasis.opendocument")
                || matches!(
                    subtype,
                    "gzip"
                        | "x-gzip"
                        | "zstd"
                        | "x-xz"
                        | "x-bzip2"
                        | "x-7z-compressed"
                        | "vnd.rar"
                        | "x-rar-compressed"
                        | "vnd.android.package-archive"
                        | "epub+zip"
                        | "java-archive"
                )
        }
        _ => false,
    }
}

#[cfg(test)]
#[path = "compress_tests.rs"]
mod tests;
