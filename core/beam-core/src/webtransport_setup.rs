//! Setting up one browser: HTTP/3 on its QUIC connection, the WebTransport session it asks
//! for at [`PATH`], and the session's first bidirectional stream.

use std::sync::Arc;

use h3::ext::Protocol;
use h3_webtransport::server::{AcceptedBi, WebTransportSession};
use http::{Method, Response, StatusCode};

use crate::Result;
use crate::webtransport::{BrowserStream, PATH, Session, network};

/// Runs HTTP/3 on `connection` until the browser asks for a WebTransport session at
/// [`PATH`], then waits for the session's first bidirectional stream.
pub(crate) async fn open_session(connection: quinn::Connection) -> Result<BrowserStream> {
    let mut h3 = h3::server::builder()
        .enable_webtransport(true)
        .enable_extended_connect(true)
        .enable_datagram(true)
        .max_webtransport_sessions(1)
        .send_grease(true)
        .build(h3_quinn::Connection::new(connection))
        .await
        .map_err(network)?;
    loop {
        let resolver = h3
            .accept()
            .await
            .map_err(network)?
            .ok_or_else(|| network("browser closed before opening a session"))?;
        let (request, mut stream) = resolver.resolve_request().await.map_err(network)?;
        let wants_session = request.method() == Method::CONNECT
            && request.extensions().get::<Protocol>() == Some(&Protocol::WEB_TRANSPORT)
            && request.uri().path() == PATH;
        if !wants_session {
            let refused = Response::builder()
                .status(StatusCode::NOT_FOUND)
                .body(())
                .map_err(network)?;
            stream.send_response(refused).await.map_err(network)?;
            stream.finish().await.map_err(network)?;
            continue;
        }
        let session = Arc::new(
            WebTransportSession::accept(request, stream, h3)
                .await
                .map_err(network)?,
        );
        return first_stream(session).await;
    }
}

async fn first_stream(session: Arc<Session>) -> Result<BrowserStream> {
    loop {
        match session.accept_bi().await.map_err(network)? {
            Some(AcceptedBi::BidiStream(_, stream)) => {
                return Ok(BrowserStream::new(stream, session));
            }
            // A plain HTTP request inside the session: not something Beam serves.
            Some(AcceptedBi::Request(_, mut stream)) => {
                let refused = Response::builder()
                    .status(StatusCode::NOT_FOUND)
                    .body(())
                    .map_err(network)?;
                stream.send_response(refused).await.map_err(network)?;
            }
            None => {
                return Err(network(
                    "browser closed its session before opening a stream",
                ));
            }
        }
    }
}
