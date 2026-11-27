//! Plays the phone app for the meeting end-to-end test, through the same `Engine` calls the
//! Android app makes: trusts the browser whose public key it is given (as if paired earlier),
//! waits for it in their room on the signaling `service`, then:
//!
//! 1. sends "from the phone again";
//! 2. prints the first item the browser sends;
//! 3. waits for the browser to stop the link.
//!
//! ```text
//! cargo run -p beam-ffi --example browser_meeting_peer -- <browser key hex> ws://127.0.0.1:8787
//! PHONE key=5f1c…
//! LINKED
//! CLIP text=from the browser again
//! ENDED clean
//! ```
#![allow(clippy::print_stdout)] // printing is how it reports to the test

use std::fmt::Write as _;

use beam_ffi::{BeamError, ClipBody, DeviceIdentity, Engine, SessionCipher};

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), BeamError> {
    let usage = || BeamError::Protocol {
        detail: "usage: browser_meeting_peer <browser key hex> <ws://signaling>".into(),
    };
    let browser = std::env::args().nth(1).ok_or_else(usage)?;
    let service = std::env::args().nth(2).ok_or_else(usage)?;
    let browser = unhex(&browser).ok_or_else(usage)?;

    let identity = DeviceIdentity::generate()?;
    println!("PHONE key={}", hex(&identity.public_key()));
    let engine = Engine::new(identity)?;
    let meeting = engine
        .meet_browser(
            browser,
            SessionCipher::ChaChaPoly,
            service,
            vec!["127.0.0.1".into()],
        )
        .await?;
    let link = meeting.meet().await?;
    println!("LINKED");

    link.send_text("from the phone again".into(), false).await?;
    match link.next_clip().await? {
        Some(clip) => match clip.body {
            ClipBody::Text { text } => println!("CLIP text={text}"),
            ClipBody::Image { .. } => println!("CLIP an image"),
        },
        None => println!("ENDED before the browser sent anything"),
    }
    match link.next_clip().await {
        Ok(None) => println!("ENDED clean"),
        Ok(Some(_)) => println!("ENDED with an unexpected item"),
        Err(error) => println!("ENDED {error}"),
    }
    Ok(())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().fold(String::new(), |mut out, byte| {
        let _ = write!(out, "{byte:02x}"); // writing to a String can't fail
        out
    })
}

fn unhex(text: &str) -> Option<Vec<u8>> {
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(text.get(i..i + 2)?, 16).ok())
        .collect()
}
