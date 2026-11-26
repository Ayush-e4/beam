//! Telling the page how far a transfer got, without a timer of its own: whenever the transfer
//! wakes for new data, the page's callback hears the byte counts (at most every
//! [`PROGRESS_EVERY_MS`], and once at the end).

use std::future::{Future, poll_fn};
use std::pin::pin;

use beam_core::Progress;
use js_sys::{Date, Function};
use wasm_bindgen::JsValue;

/// How often, at most, the page hears about progress.
const PROGRESS_EVERY_MS: f64 = 100.0;

/// Runs `future`, calling `report(done, total)` with `progress`'s byte counts as it goes. A
/// callback that throws is the page's bug; the transfer carries on.
pub async fn reporting<F: Future>(
    future: F,
    progress: &Progress,
    report: Option<&Function>,
) -> F::Output {
    let mut future = pin!(future);
    let mut last = f64::NEG_INFINITY;
    poll_fn(|cx| {
        let polled = future.as_mut().poll(cx);
        if let Some(report) = report {
            let now = Date::now();
            if polled.is_ready() || now - last >= PROGRESS_EVERY_MS {
                last = now;
                let done = JsValue::from(js_number(progress.bytes_done()));
                let total = JsValue::from(js_number(progress.bytes_total()));
                let _ = report.call2(&JsValue::NULL, &done, &total);
            }
        }
        polled
    })
    .await
}

/// A byte count as a JavaScript number, exact below 2^53 (8 PiB).
#[allow(clippy::cast_precision_loss)]
pub const fn js_number(n: u64) -> f64 {
    n as f64
}
