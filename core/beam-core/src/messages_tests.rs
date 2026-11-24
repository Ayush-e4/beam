use proptest::prelude::*;

use super::*;
use crate::manifest::PACK_ITEM_ID;

const ID: TransferId = TransferId([4; 16]);

fn range(item_id: u32, chunks: Range<u64>) -> ChunkRange {
    ChunkRange { item_id, chunks }
}

#[test]
fn accept_round_trips_with_and_without_a_selection() {
    for items in [vec![], vec![1, 5, 9]] {
        let accept = Accept {
            transfer_id: ID,
            items,
        };
        assert_eq!(Accept::decode(&accept.encode().unwrap()).unwrap(), accept);
    }
}

#[test]
fn reasons_round_trip_and_unknown_ones_fail() {
    for reason in [
        Reason::UserDeclined,
        Reason::NoSpace,
        Reason::Busy,
        Reason::Failed,
    ] {
        assert_eq!(
            decode_reason(&encode_reason(ID, reason)).unwrap(),
            (ID, reason)
        );
    }
    let mut bad = encode_reason(ID, Reason::Busy);
    bad[16] = 99;
    assert!(decode_reason(&bad).is_err());
}

#[test]
fn resume_and_ack_round_trip() {
    let have = vec![range(1, 0..4), range(2, 7..8), range(PACK_ITEM_ID, 0..1)];
    assert_eq!(
        decode_resume(&encode_resume(ID, &have).unwrap()).unwrap(),
        (ID, have.clone())
    );
    assert_eq!(decode_ack(&encode_ack(&have).unwrap()).unwrap(), have);
    assert_eq!(decode_ack(&encode_ack(&[]).unwrap()).unwrap(), vec![]);
}

#[test]
fn empty_or_reversed_ranges_are_rejected() {
    // Built field by field: a literal reversed range is exactly what we want to send.
    let reversed = Range { start: 6, end: 5 };
    for bad in [range(1, 5..5), range(1, reversed)] {
        assert!(decode_ack(&encode_ack(&[bad]).unwrap()).is_err());
    }
}

#[test]
fn chunk_header_round_trips_and_keeps_the_body() {
    let header = ChunkHeader {
        item_id: 3,
        index: 12,
        encoding: ChunkEncoding::Zstd,
        raw_len: 4_194_304,
        hash: [7; 32],
        root: Some([8; 32]),
    };
    let payload = header.encode(b"compressed body");
    let (back, body) = ChunkHeader::decode(&payload).unwrap();
    assert_eq!(back, header);
    assert_eq!(body, b"compressed body");

    let no_root = ChunkHeader {
        root: None,
        ..header
    };
    let payload = no_root.encode(&[]);
    let (back, body) = ChunkHeader::decode(&payload).unwrap();
    assert_eq!(back, no_root);
    assert_eq!(body, b"");
}

#[test]
fn truncated_or_garbled_payloads_are_rejected() {
    let accept = Accept {
        transfer_id: ID,
        items: vec![1, 2],
    }
    .encode()
    .unwrap();
    assert!(Accept::decode(&accept[..accept.len() - 1]).is_err());
    let mut trailing = accept.clone();
    trailing.push(0);
    assert!(Accept::decode(&trailing).is_err());

    let header = ChunkHeader {
        item_id: 1,
        index: 0,
        encoding: ChunkEncoding::Raw,
        raw_len: 1,
        hash: [0; 32],
        root: None,
    };
    let payload = header.encode(b"x");
    assert!(ChunkHeader::decode(&payload[..20]).is_err());
    let mut encoding = payload.clone();
    encoding[12] = 9;
    assert!(ChunkHeader::decode(&encoding).is_err());
}

proptest! {
    #[test]
    fn decoders_never_panic(payload in prop::collection::vec(any::<u8>(), 0..300)) {
        let _ = Accept::decode(&payload);
        let _ = decode_reason(&payload);
        let _ = decode_resume(&payload);
        let _ = decode_ack(&payload);
        let _ = ChunkHeader::decode(&payload);
    }

    #[test]
    fn any_ranges_round_trip(raw in prop::collection::vec((any::<u32>(), 0u64..1000, 1u64..1000), 0..50)) {
        let ranges: Vec<_> = raw.into_iter().map(|(id, s, len)| range(id, s..s + len)).collect();
        prop_assert_eq!(decode_ack(&encode_ack(&ranges).unwrap()).unwrap(), ranges);
    }
}
