//! A browser stream pair (a WebTransport stream's `readable` and `writable`) as a tokio byte
//! stream, so [`beam_core::Connection`] runs over it exactly as it does over TCP.

use std::future::Future;
use std::io;
use std::pin::Pin;
use std::task::{Context, Poll, ready};

use js_sys::{Reflect, Uint8Array};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::JsFuture;
use web_sys::{
    ReadableStream, ReadableStreamDefaultReader, WritableStream, WritableStreamDefaultWriter,
};

/// Reads and writes bytes through the browser's streams. One read and one write are in
/// flight at most, so a slow network pushes back on the engine instead of piling up memory.
pub struct JsStream {
    reader: ReadableStreamDefaultReader,
    writer: WritableStreamDefaultWriter,
    reading: Option<JsFuture>,
    unread: Vec<u8>,
    unread_at: usize,
    ended: bool,
    writing: Option<JsFuture>,
    closing: Option<JsFuture>,
}

impl JsStream {
    /// Locks both streams for this connection.
    ///
    /// # Errors
    ///
    /// The browser's error if either stream is already locked or closed.
    pub fn new(readable: &ReadableStream, writable: &WritableStream) -> Result<Self, JsValue> {
        let reader = readable
            .get_reader()
            .dyn_into::<ReadableStreamDefaultReader>()
            .map_err(JsValue::from)?;
        let writer = writable.get_writer()?;
        Ok(Self {
            reader,
            writer,
            reading: None,
            unread: Vec::new(),
            unread_at: 0,
            ended: false,
            writing: None,
            closing: None,
        })
    }

    /// Waits for the write in flight, if any, and reports its failure.
    fn poll_written(&mut self, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        if let Some(writing) = self.writing.as_mut() {
            let done = ready!(Pin::new(writing).poll(cx));
            self.writing = None;
            done.map_err(js_error)?;
        }
        Poll::Ready(Ok(()))
    }
}

impl AsyncRead for JsStream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = &mut *self;
        loop {
            if this.unread_at < this.unread.len() {
                let n = buf.remaining().min(this.unread.len() - this.unread_at);
                buf.put_slice(&this.unread[this.unread_at..this.unread_at + n]);
                this.unread_at += n;
                return Poll::Ready(Ok(()));
            }
            if this.ended {
                return Poll::Ready(Ok(())); // nothing put in `buf` means the stream ended
            }
            let reader = &this.reader;
            let reading = this
                .reading
                .get_or_insert_with(|| JsFuture::from(reader.read()));
            let result = ready!(Pin::new(reading).poll(cx));
            this.reading = None;
            let chunk = result.map_err(js_error)?;
            if field(&chunk, "done")?.as_bool() == Some(true) {
                this.ended = true;
                continue;
            }
            let bytes = field(&chunk, "value")?
                .dyn_into::<Uint8Array>()
                .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "stream gave non-bytes"))?;
            this.unread = bytes.to_vec();
            this.unread_at = 0;
        }
    }
}

impl AsyncWrite for JsStream {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        ready!(self.poll_written(cx))?;
        // The browser keeps its own copy, so `buf` is free as soon as this returns.
        let chunk = Uint8Array::from(buf);
        self.writing = Some(JsFuture::from(self.writer.write_with_chunk(&chunk)));
        Poll::Ready(Ok(buf.len()))
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        self.poll_written(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        ready!(self.poll_written(cx))?;
        let this = &mut *self;
        let writer = &this.writer;
        let closing = this
            .closing
            .get_or_insert_with(|| JsFuture::from(writer.close()));
        let closed = ready!(Pin::new(closing).poll(cx));
        closed.map_err(js_error)?;
        Poll::Ready(Ok(()))
    }
}

fn field(object: &JsValue, name: &str) -> io::Result<JsValue> {
    Reflect::get(object, &JsValue::from_str(name)).map_err(js_error)
}

/// The browser's message for a failed stream operation.
#[allow(clippy::needless_pass_by_value)] // used as `map_err(js_error)`
pub fn js_error(err: JsValue) -> io::Error {
    let message = err
        .dyn_ref::<js_sys::Error>()
        .map(|error| String::from(error.message()))
        .or_else(|| err.as_string())
        .unwrap_or_else(|| "browser stream failed".to_owned());
    io::Error::new(io::ErrorKind::ConnectionAborted, message)
}
