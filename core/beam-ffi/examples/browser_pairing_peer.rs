//! Plays the phone app for the browser pairing end-to-end test, through the same `Engine`
//! calls the Android app makes: scans the browser's code, prints the sealed offer it would post
//! in the signaling room (hex), and pairs with the browser that connects. Then it checks the
//! code can't pair a second browser.
//!
//! ```text
//! cargo run -p beam-ffi --example browser_pairing_peer -- 'BEAM1:…'
//! PHONE key=8f3a…
//! OFFER sealed=01a4…
//! PAIRED code=042 917 name=Test browser key=5c1e…
//! SPENT a browser already paired with this code
//! ```
#![allow(clippy::print_stdout)] // printing is how it reports to the test

use std::fmt::Write as _;
use std::time::Duration;

use beam_ffi::{BeamError, DeviceIdentity, Engine};

const NAME: &str = "Test phone";
const LINGER: Duration = Duration::from_secs(1);

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), BeamError> {
    let code = std::env::args().nth(1).ok_or_else(|| BeamError::Protocol {
        detail: "usage: browser_pairing_peer <code>".into(),
    })?;
    let identity = DeviceIdentity::generate()?;
    let engine = Engine::new(identity.clone())?;
    println!("PHONE key={}", hex(&identity.public_key()));

    let pairing = engine.pair_with_browser(code).await?;
    println!(
        "OFFER sealed={}",
        hex(&pairing.offer(vec!["127.0.0.1".into()])?)
    );
    let browser = pairing.wait_for_browser(NAME.into()).await?;
    println!(
        "PAIRED code={} name={} key={}",
        browser.verification_code,
        browser.name,
        hex(&browser.public_key)
    );
    match pairing.wait_for_browser(NAME.into()).await {
        Err(BeamError::Protocol { detail }) => println!("SPENT {detail}"),
        other => println!("NOT SPENT {other:?}"),
    }
    // Linger so the browser has our HELLO before the connection closes.
    tokio::time::sleep(LINGER).await;
    pairing.stop();
    Ok(())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().fold(String::new(), |mut out, byte| {
        let _ = write!(out, "{byte:02x}"); // writing to a String can't fail
        out
    })
}
