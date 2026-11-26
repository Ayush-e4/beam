//! Engine errors as JavaScript errors the web app can act on. Each is an `Error` named
//! `BeamError` with a `category` (`network`, `storage`, `permission`, `protocol`, `crypto`,
//! `cancelled`, or `invalid` for bad arguments), so the UI picks a message without parsing text.

use beam_core::{BeamError, ErrorCategory};
use js_sys::{Error, Reflect};
use wasm_bindgen::JsValue;

/// A failure inside the engine.
#[allow(clippy::needless_pass_by_value)] // used as `map_err(beam_error)`
pub fn beam_error(err: BeamError) -> JsValue {
    tagged(&err.to_string(), category_name(err.category()))
}

/// A bad argument from the web app.
pub fn invalid(message: &str) -> JsValue {
    tagged(message, "invalid")
}

fn tagged(message: &str, category: &str) -> JsValue {
    let error = Error::new(message);
    error.set_name("BeamError");
    // Setting a property on a fresh, unfrozen Error can't fail; if a browser ever refused, the
    // error still carries its message, so the result is deliberately not needed.
    let _ = Reflect::set(
        &error,
        &JsValue::from_str("category"),
        &JsValue::from_str(category),
    );
    error.into()
}

const fn category_name(category: ErrorCategory) -> &'static str {
    match category {
        ErrorCategory::Network => "network",
        ErrorCategory::Storage => "storage",
        ErrorCategory::Permission => "permission",
        ErrorCategory::Protocol => "protocol",
        ErrorCategory::Crypto => "crypto",
        ErrorCategory::Cancelled => "cancelled",
    }
}
