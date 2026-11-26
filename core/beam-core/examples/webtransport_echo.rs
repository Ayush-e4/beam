//! A WebTransport server that echoes the first stream a browser opens, for checking the phone's
//! browser path against a real browser:
//!
//! ```text
//! cargo run -p beam-core --example webtransport_echo
//! LISTENING port=54321 hash=3f9c…
//! ```
//!
//! A page then connects with
//! `new WebTransport("https://127.0.0.1:54321/beam", { serverCertificateHashes: [{ algorithm: "sha-256", value }] })`.

use std::fmt::Write as _;
use std::net::{Ipv4Addr, SocketAddr};

use beam_core::webtransport::WebTransportServer;
use beam_core::{BeamError, Result};

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    let server = WebTransportServer::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))?;
    let hash = server
        .certificate_hash()
        .iter()
        .fold(String::new(), |mut hex, byte| {
            let _ = write!(hex, "{byte:02x}");
            hex
        });
    println!("LISTENING port={} hash={hash}", server.local_addr()?.port());
    let (stream, from) = server.accept().await?;
    println!("SESSION from={from}");
    let (mut reader, mut writer) = tokio::io::split(stream);
    let echoed = tokio::io::copy(&mut reader, &mut writer)
        .await
        .map_err(BeamError::network)?;
    tokio::io::AsyncWriteExt::shutdown(&mut writer)
        .await
        .map_err(BeamError::network)?;
    println!("ECHOED bytes={echoed}");
    // Keep the stream (and so the session) until the browser closes, or the end of the echo
    // could be cut off. Chrome leaves the QUIC connection open after closing its session, so
    // this can take until the idle timeout (30 s).
    server.wait_idle().await;
    drop((reader, writer));
    Ok(())
}
