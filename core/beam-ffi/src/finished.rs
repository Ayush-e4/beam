//! Transfers this device received and saved, newest last. A sender that missed `done` asks
//! again; this list is how the engine tells it the files arrived instead of asking the person
//! a second time. It lives in memory, and in a file once the app names one
//! ([`crate::Engine::remember_finished_in`]), so it survives restarts.

use std::collections::VecDeque;
use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};

use beam_core::TransferId;

/// How many received transfers to remember.
pub(crate) const REMEMBERED: usize = 256;
const MAGIC: &[u8; 8] = b"BEAMFIN1";
const ID_LEN: usize = 16;

#[derive(Debug, Default)]
pub(crate) struct Finished(Mutex<State>);

#[derive(Debug, Default)]
struct State {
    ids: VecDeque<TransferId>,
    file: Option<PathBuf>,
}

impl Finished {
    pub(crate) fn contains(&self, id: TransferId) -> bool {
        self.state().ids.contains(&id)
    }

    /// Remembers `id`, and saves the list if it has a file. Remembered in memory even if
    /// saving fails.
    pub(crate) fn insert(&self, id: TransferId) -> io::Result<()> {
        let mut state = self.state();
        if state.ids.contains(&id) {
            return Ok(());
        }
        if state.ids.len() == REMEMBERED {
            state.ids.pop_front();
        }
        state.ids.push_back(id);
        match &state.file {
            Some(path) => write(path, &state.ids),
            None => Ok(()),
        }
    }

    /// Loads the list saved in `path`, adds what is in memory, and saves there from now on.
    pub(crate) fn keep_in(&self, path: PathBuf) -> io::Result<()> {
        let saved = read(&path)?;
        let mut state = self.state();
        let mut ids: VecDeque<TransferId> = saved.into();
        for id in &state.ids {
            if !ids.contains(id) {
                ids.push_back(*id);
            }
        }
        while ids.len() > REMEMBERED {
            ids.pop_front();
        }
        write(&path, &ids)?;
        state.ids = ids;
        state.file = Some(path);
        Ok(())
    }

    fn state(&self) -> std::sync::MutexGuard<'_, State> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

fn read(path: &Path) -> io::Result<Vec<TransferId>> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error),
    };
    let ids = bytes
        .strip_prefix(MAGIC)
        .filter(|rest| rest.len() % ID_LEN == 0)
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "damaged list of received transfers",
            )
        })?;
    Ok(ids
        .as_chunks::<ID_LEN>()
        .0
        .iter()
        .map(|id| TransferId(*id))
        .collect())
}

/// Writes the list to a temporary file, syncs it and renames it over `path`, so a crash
/// leaves the old list or the new one.
fn write(path: &Path, ids: &VecDeque<TransferId>) -> io::Result<()> {
    let temporary = path.with_extension("tmp");
    let mut file = File::create(&temporary)?;
    file.write_all(MAGIC)?;
    for id in ids {
        file.write_all(&id.0)?;
    }
    file.sync_all()?;
    fs::rename(&temporary, path)
}

#[cfg(test)]
#[path = "finished_tests.rs"]
mod tests;
