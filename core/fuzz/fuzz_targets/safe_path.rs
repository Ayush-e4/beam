//! Path sanitising never panics, and every accepted path stays inside the destination.
#![no_main]

use std::path::{Component, Path};

use beam_core::SafePath;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|raw: &str| {
    let Ok(path) = SafePath::parse(raw) else {
        return;
    };
    let destination = Path::new("/dest");
    let resolved = path.resolve_in(destination);
    assert!(
        resolved.starts_with(destination),
        "{raw:?} escaped to {resolved:?}"
    );
    let inside = resolved.strip_prefix(destination).expect("checked above");
    assert!(
        inside
            .components()
            .all(|c| matches!(c, Component::Normal(_)))
    );
});
