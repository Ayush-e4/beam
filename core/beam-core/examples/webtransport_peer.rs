//! Plays the phone for the browser engine's end-to-end tests: reads the invite a browser
//! shows, serves WebTransport, pairs as the device that scanned the code, then either echoes
//! every frame back until the browser closes, or sends files to it.
//!
//! ```text
//! cargo run -p beam-core --example webtransport_peer -- 'BEAM1:…'
//! LISTENING port=54321 hash=3f9c…
//! PAIRED code=042 917
//! ECHOED frames=4
//!
//! cargo run -p beam-core --example webtransport_peer -- 'BEAM1:…' send trip/ notes.txt
//! …
//! SENT outcome=Delivered ms=812
//! ```

use std::fmt::Write as _;
use std::net::{Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use beam_core::webtransport::WebTransportServer;
use beam_core::{
    BeamError, Connection, Handshake, PairingInvite, Result, Role, Sender, StaticKeypair,
    prepare_transfer, send_transfer,
};

/// How long to stay up after the echo, for the last acknowledgements to cross.
const LINGER: Duration = Duration::from_secs(1);

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let text = args.next().ok_or_else(|| {
        BeamError::Malformed("usage: webtransport_peer <invite> [send <path>...]".into())
    })?;
    let to_send: Vec<PathBuf> = match args.next().as_deref() {
        None => Vec::new(),
        Some("send") => args.map(PathBuf::from).collect(),
        Some(other) => return Err(BeamError::Malformed(format!("unknown mode {other}"))),
    };
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

    if to_send.is_empty() {
        echo(&mut connection).await?;
    } else {
        send(&mut connection, &to_send).await?;
    }
    // Linger so the browser's close is acknowledged. Not until idle: Chrome keeps the QUIC
    // connection open after its session ends, which would take the full idle timeout.
    let _ = tokio::time::timeout(LINGER, server.wait_idle()).await;
    Ok(())
}

/// Echoes every frame until the browser closes, then closes too.
async fn echo<S>(connection: &mut Connection<S>) -> Result<()>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let mut frames = 0;
    while let Some(frame) = connection.recv().await? {
        connection.send(&frame).await?;
        frames += 1;
    }
    connection.close().await?;
    println!("ECHOED frames={frames}");
    Ok(())
}

/// Offers `paths` and sends them if the browser accepts.
async fn send<S>(connection: &mut Connection<S>, paths: &[PathBuf]) -> Result<()>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let sender = Arc::new(Mutex::new(Sender::new(prepare_transfer(paths)?)));
    let started = Instant::now();
    let outcome = send_transfer(connection, &sender).await?;
    println!(
        "SENT outcome={outcome:?} ms={}",
        started.elapsed().as_millis()
    );
    Ok(())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().fold(String::new(), |mut out, byte| {
        let _ = write!(out, "{byte:02x}");
        out
    })
}
