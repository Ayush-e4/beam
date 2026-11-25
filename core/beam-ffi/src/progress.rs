//! Progress callbacks into the app.

use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use beam_core::Progress;
use tokio::time::{MissedTickBehavior, interval};

/// How often a running transfer reports progress, at most.
const PROGRESS_INTERVAL: Duration = Duration::from_millis(100);

/// Receives a transfer's progress. Called on an engine thread, at most every 100 ms and once
/// more when the transfer ends; keep it quick (post to the UI, don't do I/O).
#[uniffi::export(with_foreign)]
pub trait ProgressListener: Send + Sync {
    /// File bytes done so far, out of the selected total.
    fn on_progress(&self, bytes_done: u64, bytes_total: u64);
}

/// Runs `work`, reporting `progress` to `listener` whenever it changed.
pub(crate) async fn reporting<T>(
    progress: Progress,
    listener: Arc<dyn ProgressListener>,
    work: impl Future<Output = T>,
) -> T {
    let mut last = None;
    let mut report = || {
        let now = (progress.bytes_done(), progress.bytes_total());
        if last != Some(now) {
            last = Some(now);
            listener.on_progress(now.0, now.1);
        }
    };
    let mut ticks = interval(PROGRESS_INTERVAL);
    ticks.set_missed_tick_behavior(MissedTickBehavior::Skip);
    tokio::pin!(work);
    loop {
        tokio::select! {
            output = &mut work => {
                report();
                return output;
            }
            _ = ticks.tick() => report(),
        }
    }
}
