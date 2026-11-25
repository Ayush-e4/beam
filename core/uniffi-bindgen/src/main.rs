//! UniFFI's bindings generator, pinned to the same version as `beam-ffi`.
//!
//! `cargo run -p uniffi-bindgen -- generate --library <libbeam_ffi> --language kotlin --out-dir <dir>`

fn main() {
    uniffi::uniffi_bindgen_main();
}
