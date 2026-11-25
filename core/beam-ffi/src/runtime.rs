//! Runs engine work on the engine's own tokio runtime, whatever executor awaits it.
//!
//! UniFFI drives exported futures from Kotlin coroutines. Each call spawns its work on the
//! engine runtime and awaits the task; when the coroutine is cancelled UniFFI drops the
//! future, and dropping it aborts the task, which drops its connection. Transfers resume from
//! their journal on the next attempt.

use std::future::Future;

use tokio::runtime::Handle;
use tokio::task::JoinHandle;

use crate::BeamError;

/// Aborts the task when dropped, so cancelling the caller cancels the work.
struct AbortOnDrop<T>(JoinHandle<T>);

impl<T> Drop for AbortOnDrop<T> {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// Spawns `work` on `handle` and waits for it.
pub(crate) async fn run<T, F>(handle: &Handle, work: F) -> Result<T, BeamError>
where
    T: Send + 'static,
    F: Future<Output = Result<T, BeamError>> + Send + 'static,
{
    let mut task = AbortOnDrop(handle.spawn(work));
    match (&mut task.0).await {
        Ok(result) => result,
        Err(error) if error.is_cancelled() => Err(BeamError::Cancelled {
            detail: "the engine shut down".into(),
        }),
        Err(error) => Err(BeamError::Internal {
            detail: format!("engine task failed: {error}"),
        }),
    }
}

/// Runs blocking file work off the async threads.
pub(crate) async fn blocking<T, F>(work: F) -> Result<T, BeamError>
where
    T: Send + 'static,
    F: FnOnce() -> beam_core::Result<T> + Send + 'static,
{
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|error| BeamError::Internal {
            detail: format!("file task failed: {error}"),
        })?
        .map_err(BeamError::from)
}
