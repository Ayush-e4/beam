//! The little the transfer loops need from the platform: run file work without stalling the
//! network, wait with a deadline, and read a monotonic clock.
//!
//! Native builds use tokio: file work goes to its blocking pool. In a browser the engine runs
//! in a dedicated worker, never on the page's thread, so file work runs inline there (OPFS and
//! `FileReaderSync` are synchronous anyway); timers are `setTimeout` and the clock is
//! `performance.now()`.

#[cfg(all(not(feature = "native"), target_family = "wasm"))]
pub(crate) use browser::{Instant, blocking, timeout};
#[cfg(feature = "native")]
pub(crate) use native::{Instant, blocking, timeout};

#[cfg(feature = "native")]
mod native {
    use std::future::Future;
    use std::io;
    use std::time::Duration;

    pub(crate) use std::time::Instant;

    use crate::{BeamError, Result};

    /// Runs blocking file work on tokio's blocking pool.
    pub(crate) async fn blocking<T: Send + 'static>(
        work: impl FnOnce() -> Result<T> + Send + 'static,
    ) -> Result<T> {
        tokio::task::spawn_blocking(work).await.map_err(|err| {
            BeamError::storage(io::Error::other(format!("file task failed: {err}")))
        })?
    }

    /// `future`'s output, or `None` if `limit` passes first.
    pub(crate) async fn timeout<F: Future>(limit: Duration, future: F) -> Option<F::Output> {
        tokio::time::timeout(limit, future).await.ok()
    }
}

#[cfg(all(not(feature = "native"), target_family = "wasm"))]
mod browser {
    use std::future::{Future, poll_fn};
    use std::pin::{Pin, pin};
    use std::task::{Context, Poll};
    use std::time::Duration;

    use wasm_bindgen::prelude::*;
    use wasm_bindgen_futures::JsFuture;

    use crate::Result;

    // Globals in both pages and workers.
    #[wasm_bindgen]
    extern "C" {
        #[wasm_bindgen(js_name = setTimeout)]
        fn set_timeout(callback: &js_sys::Function, ms: i32) -> i32;
        #[wasm_bindgen(js_name = clearTimeout)]
        fn clear_timeout(id: i32);
        #[wasm_bindgen(js_namespace = performance, js_name = now)]
        fn performance_now() -> f64;
    }

    /// Runs file work inline: the worker has no other thread, and isn't the page's.
    #[allow(clippy::unused_async)] // the same signature as the native one
    pub(crate) async fn blocking<T>(work: impl FnOnce() -> Result<T>) -> Result<T> {
        work()
    }

    /// `future`'s output, or `None` if `limit` passes first.
    pub(crate) async fn timeout<F: Future>(limit: Duration, future: F) -> Option<F::Output> {
        let mut future = pin!(future);
        let mut sleep = Sleep::new(limit);
        poll_fn(|cx| {
            if let Poll::Ready(output) = future.as_mut().poll(cx) {
                return Poll::Ready(Some(output));
            }
            Pin::new(&mut sleep).poll(cx).map(|()| None)
        })
        .await
    }

    /// A `setTimeout`, cleared if dropped before it fires.
    struct Sleep {
        id: i32,
        fired: JsFuture,
    }

    impl Sleep {
        fn new(limit: Duration) -> Self {
            let ms = i32::try_from(limit.as_millis()).unwrap_or(i32::MAX);
            let mut id = 0;
            let promise = js_sys::Promise::new(&mut |resolve, _reject| {
                id = set_timeout(&resolve, ms);
            });
            Self {
                id,
                fired: JsFuture::from(promise),
            }
        }
    }

    impl Future for Sleep {
        type Output = ();

        fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
            // The promise only ever resolves; the value is `undefined`.
            Pin::new(&mut self.fired).poll(cx).map(|_| ())
        }
    }

    impl Drop for Sleep {
        fn drop(&mut self) {
            clear_timeout(self.id); // harmless once fired
        }
    }

    /// A monotonic clock reading, from `performance.now()`.
    #[derive(Debug, Clone, Copy)]
    pub(crate) struct Instant(f64);

    impl Instant {
        pub(crate) fn now() -> Self {
            Self(performance_now())
        }

        pub(crate) fn elapsed(self) -> Duration {
            Duration::from_secs_f64((performance_now() - self.0).max(0.0) / 1000.0)
        }
    }
}
