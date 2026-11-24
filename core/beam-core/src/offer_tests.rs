use proptest::prelude::*;

use super::*;
use crate::manifest::{MIN_CHUNK_SIZE, PACK_ITEM_ID};

const ID: TransferId = TransferId([9; 16]);

fn item(item_id: u32, path: &str, size: u64, modified: Option<i64>) -> ManifestItem {
    ManifestItem {
        item_id,
        path: SafePath::parse(path).unwrap(),
        size,
        mime: "image/jpeg".into(),
        modified_at_ms: modified,
        chunk_size: MIN_CHUNK_SIZE,
    }
}

fn manifest(items: Vec<ManifestItem>) -> Manifest {
    let total = items.iter().map(|i| i.size).sum();
    Manifest::new(ID, items, total).unwrap()
}

fn round_trip(manifest: &Manifest) -> Manifest {
    let mut assembler = OfferAssembler::new();
    let frames = encode_offer(manifest, 0).unwrap();
    let (last, rest) = frames.split_last().unwrap();
    for frame in rest {
        assert_eq!(frame.frame_type, FrameType::Offer);
        assert!(assembler.push(&frame.payload).unwrap().is_none());
    }
    assembler.push(&last.payload).unwrap().unwrap()
}

#[test]
fn small_manifest_fits_in_one_frame_and_round_trips() {
    let m = manifest(vec![
        item(1, "photos/a.jpg", 10, Some(1_732_000_000_000)),
        item(2, "notes/ünïcødé ✓.txt", 0, None),
        item(3, "deep/x/y/z.bin", 1 << 40, Some(-1)),
    ]);
    assert_eq!(encode_offer(&m, 0).unwrap().len(), 1);
    let back = round_trip(&m);
    assert_eq!(back, m);
    assert_eq!(back.fingerprint(), m.fingerprint());
}

#[test]
fn large_manifest_spans_several_frames() {
    let items = (1..=40_000)
        .map(|n| {
            item(
                n,
                &format!("folder-{}/file-number-{n}.dat", n % 97),
                1,
                None,
            )
        })
        .collect();
    let m = manifest(items);
    let frames = encode_offer(&m, 0).unwrap();
    assert!(frames.len() > 1);
    assert!(
        frames
            .iter()
            .all(|f| f.payload.len() <= OFFER_PIECE_LEN + 1)
    );
    assert_eq!(round_trip(&m), m);
}

#[test]
fn received_manifests_are_validated_as_a_whole() {
    // Hand-build an offer whose two items share an ID; decoding must reject it.
    let mut w = Writer::new();
    w.u8(1).bytes(&ID.0).u64(2).u32(2);
    for path in ["a", "b"] {
        w.u32(7)
            .u64(1)
            .u32(MIN_CHUNK_SIZE)
            .u8(0)
            .u16(1)
            .bytes(path.as_bytes())
            .u8(0);
    }
    let mut payload = vec![LAST_PIECE];
    payload.extend(w.finish());
    assert!(OfferAssembler::new().push(&payload).is_err());
}

#[test]
fn hostile_offers_are_rejected() {
    let good = encode_offer(&manifest(vec![item(1, "a", 1, None)]), 0).unwrap();
    let payload = &good[0].payload;

    assert!(OfferAssembler::new().push(&[]).is_err(), "empty frame");
    assert!(OfferAssembler::new().push(&[0b10]).is_err(), "unknown flag");
    let mut trailing = payload.clone();
    trailing.push(0);
    assert!(
        OfferAssembler::new().push(&trailing).is_err(),
        "trailing bytes"
    );
    assert!(
        OfferAssembler::new()
            .push(&payload[..payload.len() - 1])
            .is_err(),
        "truncated"
    );
    let mut version = payload.clone();
    version[1] = 2;
    assert!(
        OfferAssembler::new().push(&version).is_err(),
        "unknown version"
    );
    let mut traversal = payload.clone();
    let path_at = traversal.len() - 1 - 24 - 1;
    traversal[path_at] = b'/';
    assert!(
        OfferAssembler::new().push(&traversal).is_err(),
        "absolute path"
    );
}

#[test]
fn an_endless_stream_of_pieces_is_capped() {
    let piece = vec![0u8; OFFER_PIECE_LEN + 1];
    let mut assembler = OfferAssembler::new();
    let mut result = Ok(None);
    for _ in 0..=MAX_OFFER_LEN / OFFER_PIECE_LEN {
        result = assembler.push(&piece);
        if result.is_err() {
            break;
        }
    }
    assert!(result.is_err());
}

#[test]
fn overlong_mime_types_cannot_be_offered() {
    let mut long = item(1, "a", 1, None);
    long.mime = "x".repeat(MAX_MIME_LEN + 1);
    assert!(encode_offer(&manifest(vec![long]), 0).is_err());
    assert_ne!(PACK_ITEM_ID, 1);
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    #[test]
    fn any_valid_manifest_round_trips(
        sizes in prop::collection::vec(0u64..1 << 34, 1..50),
        modified in prop::option::of(any::<i64>()),
    ) {
        let items = sizes
            .iter()
            .enumerate()
            .map(|(n, &size)| item(u32::try_from(n).unwrap(), &format!("d/f{n}"), size, modified))
            .collect();
        let m = manifest(items);
        prop_assert_eq!(round_trip(&m), m);
    }
}
