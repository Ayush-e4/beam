//! Plays the phone app for the clipboard end-to-end test, through the same `Engine` calls the
//! Android app makes: pairs with the browser showing `code`, turns the connection into a
//! clipboard link and releases the pairing (the link must outlive it), then:
//!
//! 1. sends "from the phone";
//! 2. prints the first item the browser sends, and checks sending it back is skipped;
//! 3. sends a message with a one-time code, then a 100 KiB PNG;
//! 4. waits for the browser to stop the link.
//!
//! ```text
//! cargo run -p beam-ffi --example browser_link_peer -- 'BEAM1:…'
//! OFFER sealed=01a4…
//! LINKED
//! CLIP kind=Text sensitive=false text=from the browser
//! ECHO skipped
//! ENDED clean
//! ```
#![allow(clippy::print_stdout)] // printing is how it reports to the test

use std::fmt::Write as _;

use beam_ffi::{BeamError, ClipBody, DeviceIdentity, Engine};

/// The code message the browser must recognise.
const OTP_MESSAGE: &str = "Your OTP is 482913. Do not share it.";
/// The size of the image the browser must receive whole.
const IMAGE_LEN: usize = 100 * 1024;

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), BeamError> {
    let code = std::env::args().nth(1).ok_or_else(|| BeamError::Protocol {
        detail: "usage: browser_link_peer <code>".into(),
    })?;
    let engine = Engine::new(DeviceIdentity::generate()?)?;
    let pairing = engine.pair_with_browser(code).await?;
    println!(
        "OFFER sealed={}",
        hex(&pairing.offer(vec!["127.0.0.1".into()])?)
    );
    pairing.wait_for_browser("Test phone".into()).await?;
    let link = pairing.link().await?;
    drop(pairing);
    println!("LINKED");

    link.send_text("from the phone".into(), false).await?;
    let Some(clip) = link.next_clip().await? else {
        println!("ENDED before the browser sent anything");
        return Ok(());
    };
    let ClipBody::Text { text } = &clip.body else {
        println!("CLIP not text");
        return Ok(());
    };
    println!(
        "CLIP kind={:?} sensitive={} text={text}",
        clip.kind, clip.sensitive
    );
    let echoed = link.send_text(text.clone(), false).await?;
    println!("ECHO {}", if echoed { "sent" } else { "skipped" });

    link.send_text(OTP_MESSAGE.into(), false).await?;
    link.send_image("image/png".into(), vec![0x5A; IMAGE_LEN], false)
        .await?;
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
