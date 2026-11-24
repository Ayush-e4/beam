use std::path::Path;

use proptest::prelude::*;

use super::*;

fn parse(raw: &str) -> SafePath {
    SafePath::parse(raw).expect("path should be accepted")
}

fn rejected(raw: &str) -> bool {
    matches!(SafePath::parse(raw), Err(BeamError::UnsafePath { .. }))
}

#[test]
fn accepts_ordinary_nested_paths() {
    let path = parse("Photos/2026/beach.jpg");
    assert_eq!(path.segments(), ["Photos", "2026", "beach.jpg"]);
    assert_eq!(path.file_name(), "beach.jpg");
    assert_eq!(path.to_string(), "Photos/2026/beach.jpg");
}

#[test]
fn keeps_unicode_names() {
    assert_eq!(parse("दस्तावेज़/写真.png").to_string(), "दस्तावेज़/写真.png");
}

#[test]
fn rejects_traversal_and_absolute_paths() {
    for raw in [
        "../etc/passwd",
        "a/../../b",
        "a/./b",
        "..",
        "/etc/passwd",
        "\\\\server\\share",
        "C:\\Windows",
        "c:file.txt",
        "a//b",
        "a/",
        "",
        "a\0b",
    ] {
        assert!(rejected(raw), "{raw:?} must be rejected");
    }
}

#[test]
fn backslashes_never_act_as_separators() {
    let path = parse("folder\\..\\evil.txt");
    assert_eq!(path.segments().len(), 1);
    assert_eq!(path.file_name(), "folder_.._evil.txt");
}

#[test]
fn sanitizes_awkward_characters_and_trailing_dots() {
    assert_eq!(parse("what?:*.txt").file_name(), "what___.txt");
    assert_eq!(parse("report. . ").file_name(), "report");
    assert_eq!(parse("tab\there").file_name(), "tab_here");
}

#[test]
fn neutralizes_reserved_device_names() {
    assert_eq!(parse("CON").file_name(), "_CON");
    assert_eq!(parse("docs/nul.txt").file_name(), "_nul.txt");
    assert_eq!(parse("console.txt").file_name(), "console.txt");
}

#[test]
fn rejects_segments_that_sanitize_to_nothing() {
    assert!(rejected("..."));
    assert!(rejected("a/ . /b"));
}

#[test]
fn enforces_length_and_depth_limits() {
    assert!(rejected(&"a".repeat(MAX_SEGMENT_LEN + 1)));
    assert!(rejected(&vec!["d"; MAX_DEPTH + 1].join("/")));
    assert!(SafePath::parse(&vec!["d"; MAX_DEPTH].join("/")).is_ok());
}

#[test]
fn collision_key_ignores_case() {
    assert_eq!(
        parse("Docs/A.txt").collision_key(),
        parse("docs/a.TXT").collision_key()
    );
}

#[test]
fn resolves_inside_the_destination() {
    let dest = Path::new("/storage/Download/Beam");
    assert_eq!(
        parse("a/b.txt").resolve_in(dest),
        dest.join("a").join("b.txt")
    );
}

proptest! {
    #[test]
    fn any_accepted_path_stays_inside_the_destination(raw in "[a-zA-Z0-9./\\\\:_ -]{1,40}") {
        if let Ok(path) = SafePath::parse(&raw) {
            let dest = Path::new("/dest");
            let resolved = path.resolve_in(dest);
            prop_assert!(resolved.starts_with(dest));
            for segment in path.segments() {
                prop_assert!(segment != ".." && segment != "." && !segment.is_empty());
                prop_assert!(!segment.contains('/') && !segment.contains('\\'));
            }
        }
    }
}
