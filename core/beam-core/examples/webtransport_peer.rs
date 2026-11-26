//! Plays the phone for the browser engine's end-to-end test: reads the invite a browser
//! shows, serves WebTransport, pairs as the device that scanned the code, then echoes every
//! frame back until the browser closes.
//!
//! ```text
//! cargo run -p beam-core --example webtransport_peer -- 'BEAM1:…'
//! LISTENING port=54321 hash=3f9c…
//! PAIRED code=042 917
//! ECHOED frames=4
//! ```

use std::fmt::Write as _;
use std::net::{Ipv4Addr, SocketAddr};
use std::time::Duration;

use beam_core::webtransport::WebTransportServer;
use beam_core::{BeamError, Connection, Handshake, PairingInvite, Result, Role, StaticKeypair};

/// How long to stay up after the echo, for the last acknowledgements to cross.
const LINGER: Duration = Duration::from_secs(1);

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    let text = std::env::args()
        .nth(1)
        .ok_or_else(|| BeamError::Malformed("usage: webtransport_peer <invite>".into()))?;
    let invite = PairingInvite::decode(&text)?;
    let local = StaticKeypair::generate()?;

    let server = WebTransportServer::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))?;
    println!(
        "LISTENING port={} hash={}",
        server.local_addr()?.port(),
        hex(&server.certificate_hash())
    );
    let (stream, _) = server.accept().await?;
    let handshake = Handshake::pairing_scanner(
        &local,
        invite.cipher,
        Role::Responder,
        &invite.token,
        invite.displayer,
    )?;
    let mut connection = Connection::establish(stream, handshake).await?;
    println!("PAIRED code={}", connection.verification_code());

    let mut frames = 0;
    while let Some(frame) = connection.recv().await? {
        connection.send(&frame).await?;
        frames += 1;
    }
    connection.close().await?;
    println!("ECHOED frames={frames}");
    // Linger so the browser's close is acknowledged. Not until idle: Chrome keeps the QUIC
    // connection open after its session ends, which would take the full idle timeout.
    let _ = tokio::time::timeout(LINGER, server.wait_idle()).await;
    Ok(())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().fold(String::new(), |mut out, byte| {
        let _ = write!(out, "{byte:02x}");
        out
    })
}
