use proptest::prelude::*;

use super::*;

fn text(len: usize) -> Vec<u8> {
    b"Beam moves files between phones and computers. "
        .iter()
        .copied()
        .cycle()
        .take(len)
        .collect()
}

/// Bytes that look random to a compressor.
fn noise(len: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(len);
    let mut counter = 0u64;
    while out.len() < len {
        out.extend_from_slice(blake3::hash(&counter.to_le_bytes()).as_bytes());
        counter += 1;
    }
    out.truncate(len);
    out
}

#[test]
fn compressible_text_is_compressed_and_round_trips() {
    let data = text(256 * 1024);
    assert!(worth_compressing("text/plain", &data));
    let chunk = encode_chunk(&data, true).unwrap();
    assert_eq!(chunk.encoding, ChunkEncoding::Zstd);
    assert!(chunk.bytes.len() < data.len() / 10);
    assert_eq!(
        decode_chunk(chunk.encoding, &chunk.bytes, data.len()).unwrap(),
        data
    );
}

#[test]
fn already_compressed_formats_are_skipped_without_sampling() {
    let data = text(4096);
    for mime in [
        "image/jpeg",
        "image/heic",
        "video/mp4",
        "audio/mpeg",
        "application/zip",
        "application/vnd.android.package-archive",
        "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "APPLICATION/GZIP; charset=binary",
    ] {
        assert!(!worth_compressing(mime, &data), "{mime}");
    }
}

#[test]
fn uncompressed_media_and_unknown_types_are_sampled() {
    let data = text(4096);
    for mime in [
        "image/bmp",
        "audio/wav",
        "application/octet-stream",
        "text/csv",
        "",
    ] {
        assert!(worth_compressing(mime, &data), "{mime}");
    }
}

#[test]
fn incompressible_data_is_not_worth_it() {
    assert!(!worth_compressing(
        "application/octet-stream",
        &noise(SAMPLE_LEN)
    ));
    assert!(!worth_compressing("text/plain", &[]));
}

#[test]
fn chunks_that_would_not_shrink_stay_raw() {
    let data = noise(128 * 1024);
    let chunk = encode_chunk(&data, true).unwrap();
    assert_eq!(chunk.encoding, ChunkEncoding::Raw);
    assert_eq!(chunk.bytes, data);
}

#[test]
fn compression_off_sends_raw() {
    let data = text(1024);
    assert_eq!(
        encode_chunk(&data, false).unwrap().encoding,
        ChunkEncoding::Raw
    );
    assert_eq!(encode_chunk(&[], true).unwrap().bytes, Vec::<u8>::new());
}

#[test]
fn decompression_bombs_are_refused() {
    let bomb = zstd::bulk::compress(&vec![0u8; 8 * 1024 * 1024], LEVEL).unwrap();
    assert!(bomb.len() < 4096);
    assert!(matches!(
        decode_chunk(ChunkEncoding::Zstd, &bomb, 64 * 1024),
        Err(BeamError::Malformed(_))
    ));
}

#[test]
fn wrong_sizes_and_garbage_are_rejected() {
    let data = text(10_000);
    let chunk = encode_chunk(&data, true).unwrap();
    assert!(decode_chunk(chunk.encoding, &chunk.bytes, data.len() + 1).is_err());
    assert!(decode_chunk(ChunkEncoding::Raw, &data, data.len() - 1).is_err());
    assert!(decode_chunk(ChunkEncoding::Zstd, b"not zstd at all", 100).is_err());
}

#[test]
fn encoding_byte_round_trips() {
    for encoding in [ChunkEncoding::Raw, ChunkEncoding::Zstd] {
        assert_eq!(ChunkEncoding::try_from(encoding as u8).unwrap(), encoding);
    }
    assert!(ChunkEncoding::try_from(2).is_err());
}

proptest! {
    #[test]
    fn any_chunk_round_trips(data in prop::collection::vec(any::<u8>(), 0..50_000), compress: bool) {
        let chunk = encode_chunk(&data, compress).unwrap();
        prop_assert!(chunk.bytes.len() <= data.len());
        prop_assert_eq!(decode_chunk(chunk.encoding, &chunk.bytes, data.len()).unwrap(), data);
    }
}
