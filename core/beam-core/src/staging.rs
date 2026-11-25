//! Staging folders of unfinished incoming transfers: which one a transfer uses, whether a new
//! offer continues one, and removing the ones nobody came back for.
//!
//! A transfer stages in `<destination>/.beam-partial/<transfer id>/`, next to a `sender` file
//! holding the sending device's public key. If that folder belongs to another device, or can't
//! be used (on Android: one left by an earlier install of the app, whose files the new install
//! may not open), the transfer uses `<transfer id>.1`, `.2` and so on instead.

use std::fs::{self, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use crate::manifest::TransferId;
use crate::receiver_store::STAGING_DIR;
use crate::{BeamError, Result};

/// How many staging folders one transfer ID may have.
const SLOTS: u32 = 8;
const SENDER_FILE: &str = "sender";
pub(crate) const JOURNAL_FILE: &str = "journal";

/// What [`remove_stale`] did.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Cleanup {
    /// Unfinished transfers removed.
    pub removed: u32,
    /// Stale ones that couldn't be removed (for example, files an earlier install left).
    pub failed: u32,
}

/// Whether `destination` holds an unfinished `transfer_id` received from `sender` (a public
/// key), so a new offer of it continues what the person already accepted.
#[must_use]
pub fn continues(destination: &Path, transfer_id: TransferId, sender: &[u8]) -> bool {
    !sender.is_empty()
        && (0..SLOTS).any(|n| {
            let dir = slot(destination, transfer_id, n);
            fs::read(dir.join(SENDER_FILE)).is_ok_and(|owner| owner == sender)
                && dir.join(JOURNAL_FILE).is_file()
        })
}

/// Removes unfinished transfers in `destination` that made no progress for `max_age` before
/// `now`, freeing their space. A transfer still being received is never stale.
///
/// # Errors
///
/// A storage error if the staging folder exists but can't be listed.
pub fn remove_stale(destination: &Path, max_age: Duration, now: SystemTime) -> Result<Cleanup> {
    let root = destination.join(STAGING_DIR);
    let entries = match fs::read_dir(&root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Cleanup::default()),
        Err(error) => return Err(BeamError::storage(error)),
    };
    let mut cleanup = Cleanup::default();
    for entry in entries {
        let dir = entry.map_err(BeamError::storage)?.path();
        if !dir.is_dir() {
            continue;
        }
        let stale = last_change(&dir)
            .map(|changed| now.duration_since(changed).is_ok_and(|age| age >= max_age));
        match stale {
            Ok(false) => {}
            Ok(true) if fs::remove_dir_all(&dir).is_ok() => cleanup.removed += 1,
            Ok(true) | Err(_) => cleanup.failed += 1,
        }
    }
    // Only succeeds once it is empty; a transfer may still be using it.
    let _ = fs::remove_dir(&root);
    Ok(cleanup)
}

/// The staging folder for `transfer_id` from `sender`, created if needed.
pub(crate) fn claim(destination: &Path, transfer_id: TransferId, sender: &[u8]) -> Result<PathBuf> {
    let mut refused = None;
    for n in 0..SLOTS {
        let dir = slot(destination, transfer_id, n);
        match try_claim(&dir, sender) {
            Ok(true) => return Ok(dir),
            Ok(false) => {} // another device's
            Err(error) if error.kind() == io::ErrorKind::PermissionDenied => refused = Some(error),
            Err(error) => return Err(BeamError::storage(error)),
        }
    }
    Err(BeamError::storage(refused.unwrap_or_else(|| {
        io::Error::other("every staging folder for this transfer is taken")
    })))
}

/// Takes `dir` for `sender` if it is new or already theirs.
fn try_claim(dir: &Path, sender: &[u8]) -> io::Result<bool> {
    fs::create_dir_all(dir)?;
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(dir.join(SENDER_FILE))?;
    let mut owner = Vec::new();
    file.read_to_end(&mut owner)?;
    if owner.is_empty() {
        file.write_all(sender)?;
        file.sync_all()?;
        return Ok(true);
    }
    Ok(owner == sender)
}

fn slot(destination: &Path, transfer_id: TransferId, n: u32) -> PathBuf {
    let hex = transfer_id.0.iter().fold(String::new(), |mut out, b| {
        use std::fmt::Write as _;
        let _ = write!(out, "{b:02x}");
        out
    });
    let name = if n == 0 { hex } else { format!("{hex}.{n}") };
    destination.join(STAGING_DIR).join(name)
}

/// When anything in `dir` last changed.
fn last_change(dir: &Path) -> io::Result<SystemTime> {
    let mut newest = fs::metadata(dir)?.modified()?;
    for entry in fs::read_dir(dir)? {
        newest = newest.max(entry?.metadata()?.modified()?);
    }
    Ok(newest)
}

#[cfg(test)]
#[path = "staging_tests.rs"]
mod tests;
