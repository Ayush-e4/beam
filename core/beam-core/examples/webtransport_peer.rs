//! Plays the phone for the browser engine's end-to-end tests: reads the invite a browser
//! shows, serves WebTransport, pairs as the device that scanned the code, then either echoes
//! every frame back until the browser closes, sends files to it, or receives files from it.
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
//!
//! cargo run -p beam-core --example webtransport_peer -- 'BEAM1:…' receive inbox/ [--decline]
//! …
//! OFFERED files=3 bytes=25165824
//! RECEIVED files=3 ms=640
//! ```

use std::fmt::Write as _;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use beam_core::messages::Reason;
use beam_core::webtransport::WebTransportServer;
use beam_core::{
    BeamError, Connection, Handshake, PairingInvite, Receiver, Result, Role, Sender, StaticKeypair,
    prepare_transfer, read_offer, receive_transfer, reject_offer, send_transfer,
};

/// How long to stay up after the echo, for the last acknowledgements to cross.
const LINGER: Duration = Duration::from_secs(1);

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let usage = "usage: webtransport_peer <invite> [send <path>... | receive <dir> [--decline]]";
    let text = args
        .next()
        .ok_or_else(|| BeamError::Malformed(usage.into()))?;
    let mode = match args.next().as_deref() {
        None => Mode::Echo,
        Some("send") => Mode::Send(args.map(PathBuf::from).collect()),
        Some("receive") => {
            let dir = args
                .next()
                .ok_or_else(|| BeamError::Malformed(usage.into()))?;
            let decline = args.next().as_deref() == Some("--decline");
            Mode::Receive(PathBuf::from(dir), decline)
        }
        Some(_) => return Err(BeamError::Malformed(usage.into())),
    };
    let invite = PairingInvite::decode(&text)?;
    let local = StaticKeypair::generate()?;

    // BEAM_PEER_ADDR=0.0.0.0 serves the local network, to run this on a phone over Wi-Fi.
    let ip = match std::env::var("BEAM_PEER_ADDR") {
        Ok(addr) => addr
            .parse::<IpAddr>()
            .map_err(|_| BeamError::Malformed(format!("BEAM_PEER_ADDR {addr} isn't an IP")))?,
        Err(_) => IpAddr::V4(Ipv4Addr::LOCALHOST),
    };
    let server = WebTransportServer::bind(SocketAddr::from((ip, 0)))?;
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

    match mode {
        Mode::Echo => echo(&mut connection).await?,
        Mode::Send(paths) => send(&mut connection, &paths).await?,
        Mode::Receive(dir, decline) => receive(&mut connection, &dir, decline).await?,
    }
    // Linger so the browser's close is acknowledged. Not until idle: Chrome keeps the QUIC
    // connection open after its session ends, which would take the full idle timeout.
    let _ = tokio::time::timeout(LINGER, server.wait_idle()).await;
    Ok(())
}

/// What to do once paired.
enum Mode {
    Echo,
    Send(Vec<PathBuf>),
    /// Into the folder; or decline the offer.
    Receive(PathBuf, bool),
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

/// Receives what the browser offers into `dir`, or declines it.
async fn receive<S>(connection: &mut Connection<S>, dir: &Path, decline: bool) -> Result<()>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let manifest = read_offer(connection).await?;
    let bytes: u64 = manifest.items().iter().map(|item| item.size).sum();
    println!("OFFERED files={} bytes={bytes}", manifest.items().len());
    if decline {
        let transfer_id = manifest.transfer_id();
        return reject_offer(connection, transfer_id, Reason::UserDeclined).await;
    }
    let started = Instant::now();
    let saved = receive_transfer(connection, Receiver::open(manifest, dir, &[])?).await?;
    println!(
        "RECEIVED files={} ms={}",
        saved.len(),
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
