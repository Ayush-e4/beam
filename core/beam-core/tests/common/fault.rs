//! A byte stream that breaks on purpose: it stalls now and then and cuts the connection after a
//! budget of bytes written or read, often in the middle of an encrypted record.

use std::io;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::task::{Context, Poll, ready};
use std::time::Duration;

use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::time::{Sleep, sleep};

/// Small deterministic random source (xorshift64*), so a failing run can be replayed.
#[derive(Debug, Clone)]
pub struct Rng(u64);

impl Rng {
    /// Seeds are scrambled first (splitmix64), so nearby seeds give unrelated sequences.
    pub fn new(seed: u64) -> Self {
        let mut z = seed.wrapping_add(0x9E37_79B9_7F4A_7C15);
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        Self((z ^ (z >> 31)).max(1))
    }

    pub fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// A number in `range`.
    pub fn between(&mut self, range: std::ops::Range<u64>) -> u64 {
        range.start + self.next() % (range.end - range.start)
    }

    /// True with probability `percent` / 100.
    pub fn percent(&mut self, percent: u64) -> bool {
        self.next() % 100 < percent
    }
}

/// Wraps a stream; it fails once `budget` bytes have gone out or `read_budget` bytes came in.
pub struct Faulty<S> {
    inner: S,
    budget: u64,
    read_budget: u64,
    rng: Rng,
    stall: Option<Pin<Box<Sleep>>>,
    written: Arc<AtomicU64>,
}

impl<S> Faulty<S> {
    /// `written` collects how many bytes this stream sent, across attempts.
    pub fn new(inner: S, budget: u64, rng: Rng, written: Arc<AtomicU64>) -> Self {
        Self {
            inner,
            budget,
            read_budget: u64::MAX,
            rng,
            stall: None,
            written,
        }
    }

    /// Also fail once `read_budget` bytes have been read.
    pub fn failing_reads_after(mut self, read_budget: u64) -> Self {
        self.read_budget = read_budget;
        self
    }
}

fn injected() -> io::Error {
    io::Error::new(io::ErrorKind::ConnectionReset, "injected disconnect")
}

impl<S: AsyncWrite + Unpin> AsyncWrite for Faulty<S> {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        let this = self.get_mut();
        if this.stall.is_none() && this.rng.percent(2) {
            let pause = Duration::from_millis(this.rng.between(1..40));
            this.stall = Some(Box::pin(sleep(pause)));
        }
        if let Some(stall) = &mut this.stall {
            ready!(stall.as_mut().poll(cx));
            this.stall = None;
        }
        if this.budget == 0 {
            return Poll::Ready(Err(injected()));
        }
        let allowed = buf
            .len()
            .min(usize::try_from(this.budget).unwrap_or(usize::MAX));
        let written = ready!(Pin::new(&mut this.inner).poll_write(cx, &buf[..allowed]))?;
        this.budget -= written as u64;
        this.written.fetch_add(written as u64, Ordering::Relaxed);
        Poll::Ready(Ok(written))
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().inner).poll_flush(cx)
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        if this.budget == 0 {
            return Poll::Ready(Err(injected()));
        }
        Pin::new(&mut this.inner).poll_shutdown(cx)
    }
}

impl<S: AsyncRead + Unpin> AsyncRead for Faulty<S> {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        if this.read_budget == 0 {
            return Poll::Ready(Err(injected()));
        }
        let before = buf.filled().len();
        ready!(Pin::new(&mut this.inner).poll_read(cx, buf))?;
        let read = (buf.filled().len() - before) as u64;
        this.read_budget = this.read_budget.saturating_sub(read);
        Poll::Ready(Ok(()))
    }
}
