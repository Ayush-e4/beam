//! What each command does. Every command reports through `out`, one line per event: a word
//! in capitals, then `key=value` pairs, so scripts can follow along. Every command fails the
//! same way: with an [`Outcome`] error already worded for the person.

// Each command's errors are the one kind `Outcome` documents.
#![allow(clippy::missing_errors_doc)]

use std::net::UdpSocket;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use beam_ffi::{
    DeclineReason, Engine, IncomingOffer, ProgressListener, SendResult, SessionCipher, TrustedKeys,
};
use tokio::sync::mpsc::UnboundedSender;

use crate::home::{Home, hex};

/// Where a command's lines go.
pub type Out = UnboundedSender<String>;

/// A command's failure, already worded for the person.
pub type Outcome = Result<(), String>;

fn say(out: &Out, line: String) {
    // Nobody listening any more is fine: the command was asked to stop.
    let _ = out.send(line);
}

fn engine(home: &Home) -> Result<Arc<Engine>, String> {
    let identity = home
        .identity()
        .map_err(|e| format!("can't load the identity: {e}"))?;
    Engine::new(identity).map_err(|e| e.to_string())
}

/// `beam id`: this device's ID and public key.
pub fn id(home: &Home, out: &Out) -> Outcome {
    let identity = home
        .identity()
        .map_err(|e| format!("can't load the identity: {e}"))?;
    say(
        out,
        format!(
            "ID device={} key={}",
            identity.device_id(),
            hex(&identity.public_key())
        ),
    );
    Ok(())
}

/// `beam devices`: the trusted devices.
pub fn devices(home: &Home, out: &Out) -> Outcome {
    for device in home.trusted().map_err(|e| e.to_string())? {
        say(
            out,
            format!("DEVICE id={} name={}", device.device_id, device.name),
        );
    }
    Ok(())
}

/// `beam pair`: shows a pairing code for `addresses` (this Mac's main address when empty),
/// waits for a device to pair and trusts it.
pub async fn pair(
    home: &Home,
    name: String,
    addresses: Vec<String>,
    cipher: SessionCipher,
    out: &Out,
) -> Outcome {
    let engine = engine(home)?;
    let host = engine
        .host_pairing(cipher)
        .await
        .map_err(|e| e.to_string())?;
    let addresses = if addresses.is_empty() {
        vec![main_address()]
    } else {
        addresses
    };
    let code = host.code(addresses).map_err(|e| e.to_string())?;
    say(out, format!("CODE {code}"));
    let device = host
        .wait_for_device(name)
        .await
        .map_err(|e| e.to_string())?;
    home.trust(&device)
        .map_err(|e| format!("paired, but can't save it: {e}"))?;
    say(
        out,
        format!(
            "PAIRED id={} verify={} name={}",
            device.device_id, device.verification_code, device.name
        ),
    );
    Ok(())
}

/// `beam pair-with CODE`: pairs with the device showing `code` and trusts it.
pub async fn pair_with(home: &Home, code: String, name: String, out: &Out) -> Outcome {
    let engine = engine(home)?;
    let device = engine
        .pair_with(code, name, None)
        .await
        .map_err(|e| e.to_string())?;
    home.trust(&device)
        .map_err(|e| format!("paired, but can't save it: {e}"))?;
    say(
        out,
        format!(
            "PAIRED id={} verify={} name={}",
            device.device_id, device.verification_code, device.name
        ),
    );
    Ok(())
}

/// `beam send WHO HOST PORT FILE...`: sends to the trusted device `who`.
pub async fn send(
    home: &Home,
    who: &str,
    host: String,
    port: u16,
    files: Vec<String>,
    out: &Out,
) -> Outcome {
    let device = home.find(who).map_err(|e| e.to_string())?;
    let engine = engine(home)?;
    let outgoing = engine
        .prepare_send(files)
        .await
        .map_err(|e| e.to_string())?;
    let summary = outgoing.summary();
    let started = Instant::now();
    let result = outgoing
        .send_to(
            host,
            port,
            device.public_key,
            device.cipher,
            Arc::new(Quiet),
            None,
        )
        .await
        .map_err(|e| format!("couldn't send: {e}"))?;
    let seconds = started.elapsed().as_secs_f64();
    match result {
        SendResult::Delivered => say(
            out,
            format!(
                "SENT transfer={} files={} bytes={} secs={seconds:.2}",
                summary.transfer_id,
                summary.items.len(),
                summary.total_bytes
            ),
        ),
        SendResult::Declined { reason } => {
            say(
                out,
                format!(
                    "DECLINED transfer={} reason={reason:?}",
                    summary.transfer_id
                ),
            );
        }
    }
    Ok(())
}

/// `beam receive`: listens on `port` until stopped, saving offers from trusted devices (from
/// anyone with `accept_all`) into `into` and declining the rest.
pub async fn receive(
    home: &Home,
    port: u16,
    into: PathBuf,
    accept_all: bool,
    cipher: SessionCipher,
    out: &Out,
) -> Outcome {
    let engine = engine(home)?;
    let finished = home.file("finished.bin").to_string_lossy().into_owned();
    engine
        .remember_finished_in(finished)
        .await
        .map_err(|e| e.to_string())?;
    let keys = home.trusted().map_err(|e| e.to_string())?;
    let trusted = Arc::new(Keys(
        keys.into_iter().map(|device| device.public_key).collect(),
    ));
    let listener = engine
        .listen(port, cipher, trusted)
        .await
        .map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&into).map_err(|e| format!("can't use {}: {e}", into.display()))?;
    say(
        out,
        format!(
            "LISTENING port={} address={}",
            listener.port(),
            main_address()
        ),
    );
    loop {
        let offer = listener.next_offer().await.map_err(|e| e.to_string())?;
        let take = accept_all || offer.from_trusted_device();
        tokio::spawn(answer(offer, take, into.clone(), out.clone()));
    }
}

async fn answer(offer: Arc<IncomingOffer>, take: bool, into: PathBuf, out: Out) {
    let summary = offer.summary();
    let from = offer.peer_device_id();
    if !take {
        let declined = offer.decline(DeclineReason::UserDeclined).await;
        say(
            &out,
            format!(
                "DECLINED transfer={} from={from} ok={}",
                summary.transfer_id,
                declined.is_ok()
            ),
        );
        return;
    }
    let started = Instant::now();
    let destination = into.to_string_lossy().into_owned();
    match offer.accept(destination, Vec::new(), Arc::new(Quiet)).await {
        Ok(saved) => say(
            &out,
            format!(
                "RECEIVED transfer={} from={from} files={} bytes={} secs={:.2}",
                summary.transfer_id,
                saved.len(),
                summary.total_bytes,
                started.elapsed().as_secs_f64()
            ),
        ),
        Err(error) => say(
            &out,
            format!(
                "FAILED transfer={} from={from} error={error}",
                summary.transfer_id
            ),
        ),
    }
}

/// The address other devices on the network reach this one at: the one the system would use
/// to go out (no packet is sent). Loopback when there is no network.
fn main_address() -> String {
    UdpSocket::bind("0.0.0.0:0")
        .and_then(|socket| {
            socket.connect("192.0.2.1:9")?;
            socket.local_addr()
        })
        .map_or_else(|_| "127.0.0.1".into(), |addr| addr.ip().to_string())
}

struct Keys(Vec<Vec<u8>>);

impl TrustedKeys for Keys {
    fn is_trusted(&self, public_key: Vec<u8>) -> bool {
        self.0.contains(&public_key)
    }
}

struct Quiet;

impl ProgressListener for Quiet {
    fn on_progress(&self, _bytes_done: u64, _bytes_total: u64) {}
}
