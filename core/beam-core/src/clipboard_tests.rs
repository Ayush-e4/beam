use proptest::prelude::*;

use super::*;

fn png(len: usize) -> ClipItem {
    ClipItem::image("image/png", vec![7; len]).unwrap()
}

#[test]
fn text_and_images_round_trip_with_their_flags() {
    let items = [
        ClipItem::text("hello from the phone").unwrap(),
        ClipItem::text("hunter2").unwrap().with_sensitive(true),
        ClipItem::text("naïve ✓ 🚀").unwrap(),
        png(1024),
        ClipItem::image("image/jpeg", vec![1, 2, 3]).unwrap(),
    ];
    for item in items {
        let frame = item.to_frame().unwrap();
        assert_eq!(frame.frame_type, FrameType::Clipboard);
        assert_eq!(frame.stream_id, CLIPBOARD_STREAM);
        assert_eq!(ClipItem::decode(&frame.payload).unwrap(), item);
    }
}

#[test]
fn limits_hold_when_building_and_when_decoding() {
    assert!(ClipItem::text("").is_err());
    assert!(ClipItem::text("x".repeat(MAX_TEXT_LEN)).is_ok());
    assert!(ClipItem::text("x".repeat(MAX_TEXT_LEN + 1)).is_err());
    assert!(ClipItem::image("image/png", vec![]).is_err());
    assert!(ClipItem::image("image/png", vec![0; MAX_IMAGE_LEN]).is_ok());
    assert!(ClipItem::image("image/png", vec![0; MAX_IMAGE_LEN + 1]).is_err());
    assert!(ClipItem::image("image/svg+xml", vec![1]).is_err());
    assert!(ClipItem::image("text/plain", vec![1]).is_err());

    let mut too_long = vec![KIND_TEXT, 0, 10];
    too_long.extend_from_slice(b"text/plain");
    too_long.resize(too_long.len() + MAX_TEXT_LEN + 1, b'x');
    assert!(ClipItem::decode(&too_long).is_err());
}

#[test]
fn bad_payloads_are_rejected() {
    let good = ClipItem::text("hi").unwrap().encode();
    let cases: [(&str, Vec<u8>); 7] = [
        ("empty", vec![]),
        ("unknown kind", [&[9], &good[1..]].concat()),
        ("unknown flag", [&good[..1], &[2], &good[2..]].concat()),
        ("truncated type", good[..5].to_vec()),
        ("text with an image type", {
            let mut p = vec![KIND_TEXT, 0, 9];
            p.extend_from_slice(b"image/pngabc");
            p
        }),
        (
            "text that isn't UTF-8",
            [&good[..13], &[0xFF, 0xFE]].concat(),
        ),
        ("image without bytes", {
            let mut p = vec![KIND_IMAGE, 0, 9];
            p.extend_from_slice(b"image/png");
            p
        }),
    ];
    for (name, payload) in cases {
        assert!(ClipItem::decode(&payload).is_err(), "{name}");
    }
}

#[test]
fn codes_count_as_sensitive_and_images_are_labelled() {
    assert!(ClipItem::text("482913").unwrap().is_sensitive());
    assert!(!ClipItem::text("hello").unwrap().is_sensitive());
    assert!(
        ClipItem::text("hello")
            .unwrap()
            .with_sensitive(true)
            .is_sensitive()
    );
    assert_eq!(png(3).label(), ClipLabel::Image);
    assert_eq!(
        ClipItem::text("https://beam.app").unwrap().label(),
        ClipLabel::Link
    );
}

#[test]
fn fingerprints_follow_the_content_not_the_flags() {
    let hi = ClipItem::text("hi").unwrap();
    assert_eq!(
        hi.fingerprint(),
        hi.clone().with_sensitive(true).fingerprint()
    );
    assert_ne!(
        hi.fingerprint(),
        ClipItem::text("hi!").unwrap().fingerprint()
    );
    let jpeg = ClipItem::image("image/jpeg", vec![7; 3]).unwrap();
    assert_ne!(png(3).fingerprint(), jpeg.fingerprint());
}

#[test]
fn the_echo_guard_drops_what_just_arrived_and_repeats() {
    let mut guard = EchoGuard::new();
    let from_peer = ClipItem::text("from the browser").unwrap();
    guard.received(&from_peer);
    assert!(!guard.should_send(&from_peer), "an echo of what arrived");

    let copied = ClipItem::text("copied here").unwrap();
    assert!(guard.should_send(&copied));
    assert!(!guard.should_send(&copied), "the same copy again");
    assert!(
        guard.should_send(&from_peer),
        "copied again after something else"
    );
}

proptest! {
    #[test]
    fn decoding_never_panics(payload in proptest::collection::vec(any::<u8>(), 0..256)) {
        let _ = ClipItem::decode(&payload);
    }

    #[test]
    fn any_text_round_trips(text in "\\PC{1,200}", sensitive in any::<bool>()) {
        let item = ClipItem::text(text).unwrap().with_sensitive(sensitive);
        prop_assert_eq!(ClipItem::decode(&item.encode()).unwrap(), item);
    }
}
