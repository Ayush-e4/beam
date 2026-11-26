//! `beam`: see [`USAGE`].

use std::path::PathBuf;
use std::process::ExitCode;

use beam_cli::args::Args;
use beam_cli::commands::{self, Outcome};
use beam_cli::home::Home;
use beam_ffi::SessionCipher;
use tokio::sync::mpsc;

const USAGE: &str = "\
usage: beam <command> [options]

  id                                  this device's ID and public key
  devices                             the trusted devices
  pair [--name N] [--address IP]...   show a pairing code and wait for a device
  pair-with CODE [--name N]           pair with the device showing CODE
  receive [--port P] [--into DIR] [--accept-all]
                                      receive until stopped (default port 47123, ./inbox)
  send WHO HOST PORT FILE...          send to a trusted device (by name or ID)

  --cipher aes-gcm|chacha             for pair and receive (default aes-gcm)
  State lives in $BEAM_HOME, or ~/.beam-cli.";

#[tokio::main]
async fn main() -> ExitCode {
    let (out, mut lines) = mpsc::unbounded_channel();
    let printer = tokio::spawn(async move {
        while let Some(line) = lines.recv().await {
            println!("{line}");
        }
    });
    let outcome = run(out).await;
    // The command's sender is gone, so the printer ends once every line is out.
    let _ = printer.await;
    match outcome {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("beam: {message}");
            ExitCode::FAILURE
        }
    }
}

async fn run(out: commands::Out) -> Outcome {
    let args = Args::parse(std::env::args().skip(1))?;
    let home = Home::from_env().map_err(|e| e.to_string())?;
    let name = args.option("name").unwrap_or("Beam CLI").to_owned();
    let cipher = match args.option("cipher").unwrap_or("aes-gcm") {
        "aes-gcm" => SessionCipher::AesGcm,
        "chacha" => SessionCipher::ChaChaPoly,
        other => return Err(format!("unknown cipher {other}")),
    };
    match (args.command.as_str(), args.positional.as_slice()) {
        ("id", []) => commands::id(&home, &out),
        ("devices", []) => commands::devices(&home, &out),
        ("pair", []) => {
            commands::pair(&home, name, args.all("address").to_vec(), cipher, &out).await
        }
        ("pair-with", [code]) => commands::pair_with(&home, code.clone(), name, &out).await,
        ("receive", []) => {
            let port = args.port("port", 47123)?;
            let into = PathBuf::from(args.option("into").unwrap_or("inbox"));
            commands::receive(&home, port, into, args.has("accept-all"), cipher, &out).await
        }
        ("send", [who, host, port, files @ ..]) if !files.is_empty() => {
            let port = port.parse().map_err(|_| format!("{port} is not a port"))?;
            commands::send(&home, who, host.clone(), port, files.to_vec(), &out).await
        }
        _ => Err(USAGE.into()),
    }
}
