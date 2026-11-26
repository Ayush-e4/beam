//! Where the CLI keeps its state: `$BEAM_HOME`, or `~/.beam-cli`. It holds this device's
//! identity (a plain private key file, readable only by you: this is a development tool, not
//! the keystore-backed phone app), the trusted devices and the list of received transfers.

use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use beam_ffi::{DeviceIdentity, PairedDevice, SessionCipher};

/// The state folder.
#[derive(Debug, Clone)]
pub struct Home(PathBuf);

/// A device paired with this one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Trusted {
    /// Its device ID.
    pub device_id: String,
    /// The name it gave when pairing.
    pub name: String,
    /// The key it must prove on every connection.
    pub public_key: Vec<u8>,
    /// The cipher chosen when pairing.
    pub cipher: SessionCipher,
}

impl Home {
    /// `$BEAM_HOME`, or `~/.beam-cli`.
    ///
    /// # Errors
    ///
    /// If neither `BEAM_HOME` nor `HOME` is set.
    pub fn from_env() -> io::Result<Self> {
        let dir = match std::env::var_os("BEAM_HOME") {
            Some(dir) => PathBuf::from(dir),
            None => std::env::var_os("HOME")
                .map(|home| PathBuf::from(home).join(".beam-cli"))
                .ok_or_else(|| io::Error::other("set BEAM_HOME or HOME"))?,
        };
        Ok(Self(dir))
    }

    /// A state folder at `dir`.
    #[must_use]
    pub fn at(dir: impl Into<PathBuf>) -> Self {
        Self(dir.into())
    }

    /// A file inside the folder.
    #[must_use]
    pub fn file(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }

    /// This device's identity, created on first use.
    ///
    /// # Errors
    ///
    /// If the key file can't be read, written, or holds a damaged key.
    pub fn identity(&self) -> io::Result<Arc<DeviceIdentity>> {
        let path = self.file("identity.key");
        match fs::read(&path) {
            Ok(key) => DeviceIdentity::from_private_key(key).map_err(io::Error::other),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                let identity = DeviceIdentity::generate().map_err(io::Error::other)?;
                fs::create_dir_all(&self.0)?;
                let mut file = OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .mode(0o600)
                    .open(&path)?;
                file.write_all(&identity.private_key())?;
                Ok(identity)
            }
            Err(error) => Err(error),
        }
    }

    /// The trusted devices, oldest first.
    ///
    /// # Errors
    ///
    /// If the list can't be read or a line is damaged.
    pub fn trusted(&self) -> io::Result<Vec<Trusted>> {
        let text = match fs::read_to_string(self.file("trusted.txt")) {
            Ok(text) => text,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(error),
        };
        text.lines()
            .filter(|line| !line.is_empty())
            .map(parse)
            .collect()
    }

    /// Trusts `device`, replacing an earlier pairing with it.
    ///
    /// # Errors
    ///
    /// If the list can't be read or written.
    pub fn trust(&self, device: &PairedDevice) -> io::Result<()> {
        let mut all = self.trusted()?;
        all.retain(|known| known.device_id != device.device_id);
        all.push(Trusted {
            device_id: device.device_id.clone(),
            name: device.name.replace(['\t', '\n', '\r'], " "),
            public_key: device.public_key.clone(),
            cipher: device.cipher,
        });
        let text: String = all.iter().map(line).collect();
        fs::create_dir_all(&self.0)?;
        write_replacing(&self.file("trusted.txt"), text.as_bytes())
    }

    /// The trusted device called `who` (its name, or its device ID).
    ///
    /// # Errors
    ///
    /// If the list can't be read, or no trusted device is called that.
    pub fn find(&self, who: &str) -> io::Result<Trusted> {
        self.trusted()?
            .into_iter()
            .rev()
            .find(|device| device.name == who || device.device_id == who)
            .ok_or_else(|| io::Error::other(format!("no trusted device called {who}")))
    }
}

fn line(device: &Trusted) -> String {
    let cipher = match device.cipher {
        SessionCipher::AesGcm => "aes-gcm",
        SessionCipher::ChaChaPoly => "chacha",
    };
    format!(
        "{}\t{cipher}\t{}\t{}\n",
        device.device_id,
        hex(&device.public_key),
        device.name
    )
}

fn parse(line: &str) -> io::Result<Trusted> {
    let damaged = || io::Error::other(format!("damaged trusted device: {line}"));
    let mut fields = line.splitn(4, '\t');
    let (Some(device_id), Some(cipher), Some(key), Some(name)) =
        (fields.next(), fields.next(), fields.next(), fields.next())
    else {
        return Err(damaged());
    };
    let cipher = match cipher {
        "aes-gcm" => SessionCipher::AesGcm,
        "chacha" => SessionCipher::ChaChaPoly,
        _ => return Err(damaged()),
    };
    Ok(Trusted {
        device_id: device_id.to_owned(),
        name: name.to_owned(),
        public_key: unhex(key).ok_or_else(damaged)?,
        cipher,
    })
}

/// Lowercase hex.
#[must_use]
pub fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;

    bytes.iter().fold(String::new(), |mut text, byte| {
        // Writing to a String can't fail.
        let _ = write!(text, "{byte:02x}");
        text
    })
}

fn unhex(text: &str) -> Option<Vec<u8>> {
    if !text.len().is_multiple_of(2) {
        return None;
    }
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(text.get(i..i + 2)?, 16).ok())
        .collect()
}

/// Writes a temporary file and renames it over `path`, so a crash keeps the old list.
fn write_replacing(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let temporary = path.with_extension("tmp");
    fs::write(&temporary, bytes)?;
    fs::rename(temporary, path)
}

#[cfg(test)]
#[path = "home_tests.rs"]
mod tests;
