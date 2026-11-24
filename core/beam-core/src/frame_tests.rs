use proptest::prelude::*;

use super::*;
use crate::ErrorCategory;

fn encode(frame: &Frame) -> Vec<u8> {
    let mut out = Vec::new();
    frame.encode_into(&mut out).expect("valid frame encodes");
    out
}

#[test]
fn header_layout_is_type_stream_length_big_endian() {
    let frame = Frame::new(FrameType::Ping, 0x0102_0304, vec![0xAA, 0xBB]).expect("valid");
    assert_eq!(encode(&frame), [0x0A, 1, 2, 3, 4, 0, 0, 0, 2, 0xAA, 0xBB]);
}

#[test]
fn partial_header_asks_for_the_rest() {
    assert_eq!(
        Frame::decode(&[0x01, 0, 0]).expect("decodes"),
        Decoded::NeedMore(6)
    );
    assert_eq!(
        Frame::decode(&[]).expect("decodes"),
        Decoded::NeedMore(HEADER_LEN)
    );
}

#[test]
fn partial_payload_asks_for_the_rest() {
    let bytes = encode(&Frame::new(FrameType::Chunk, 7, vec![1; 10]).expect("valid"));
    assert_eq!(
        Frame::decode(&bytes[..12]).expect("decodes"),
        Decoded::NeedMore(7)
    );
}

#[test]
fn back_to_back_frames_decode_one_at_a_time() {
    let first = Frame::new(FrameType::Hello, 0, b"hi".to_vec()).expect("valid");
    let second = Frame::new(FrameType::Pong, 3, Vec::new()).expect("valid");
    let mut buf = encode(&first);
    buf.extend(encode(&second));

    let Decoded::Frame { frame, consumed } = Frame::decode(&buf).expect("decodes") else {
        panic!("expected a complete frame");
    };
    assert_eq!(frame, first);
    let Decoded::Frame { frame, .. } = Frame::decode(&buf[consumed..]).expect("decodes") else {
        panic!("expected a complete frame");
    };
    assert_eq!(frame, second);
}

#[test]
fn unknown_frame_type_is_a_protocol_error() {
    let err = Frame::decode(&[0xFF, 0, 0, 0, 0, 0, 0, 0, 0]).expect_err("must fail");
    assert_eq!(err.category(), ErrorCategory::Protocol);
}

#[test]
fn oversized_length_is_rejected_before_allocating() {
    let mut header = vec![FrameType::Chunk as u8, 0, 0, 0, 1];
    header.extend_from_slice(&u32::MAX.to_be_bytes());
    let err = Frame::decode(&header).expect_err("must fail");
    assert_eq!(err.category(), ErrorCategory::Protocol);
}

#[test]
fn oversized_payload_cannot_be_constructed() {
    let result = Frame::new(FrameType::Chunk, 1, vec![0; MAX_PAYLOAD_LEN + 1]);
    assert!(result.is_err());
}

fn any_frame_type() -> impl Strategy<Value = FrameType> {
    (0x01u8..=0x0C).prop_map(|v| FrameType::try_from(v).expect("in range"))
}

proptest! {
    #[test]
    fn encode_then_decode_round_trips(
        frame_type in any_frame_type(),
        stream_id in any::<u32>(),
        payload in proptest::collection::vec(any::<u8>(), 0..4096),
    ) {
        let frame = Frame::new(frame_type, stream_id, payload).expect("valid");
        let bytes = encode(&frame);
        let decoded = Frame::decode(&bytes).expect("decodes");
        prop_assert_eq!(decoded, Decoded::Frame { frame, consumed: bytes.len() });
    }

    #[test]
    fn decoding_arbitrary_bytes_never_panics(bytes in proptest::collection::vec(any::<u8>(), 0..64)) {
        let _ = Frame::decode(&bytes);
    }
}
