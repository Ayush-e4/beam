//! Files handed over already open, sent to the real receiver in-process.

use std::fs::{self, File, OpenOptions};
use std::io::Write;

use tempfile::TempDir;

use super::*;
use crate::messages::Accept;
use crate::{BeamError, Receiver, Sender};

const ID: TransferId = TransferId([5; 16]);

/// `bytes` in a new file of its own, opened and named `name`.
fn opened(dir: &TempDir, name: &str, bytes: &[u8]) -> OpenedFile {
    let path = tempfile::NamedTempFile::new_in(dir.path())
        .unwrap()
        .into_temp_path()
        .keep()
        .unwrap();
    fs::write(&path, bytes).unwrap();
    OpenedFile {
        name: name.to_owned(),
        file: File::open(&path).unwrap(),
    }
}

/// Sends everything to a receiver in `dest` and finishes it.
fn send_all(prepared: PreparedTransfer, dest: &std::path::Path) {
    let mut receiver = Receiver::open(prepared.manifest().clone(), dest, &[]).unwrap();
    let mut sender = Sender::new(prepared);
    sender
        .accept(&Accept {
            transfer_id: ID,
            items: Vec::new(),
        })
        .unwrap();
    loop {
        while sender.can_send() {
            let Some(payload) = sender.next_chunk().unwrap() else {
                break;
            };
            receiver.handle_chunk(&payload).unwrap();
        }
        let acks = receiver.checkpoint().unwrap();
        if acks.is_empty() {
            break;
        }
        sender.handle_ack(&acks);
    }
    assert!(sender.is_complete());
    receiver.finish().unwrap();
}

#[test]
fn open_files_arrive_under_their_names_even_without_a_path() {
    let dir = TempDir::new().unwrap();
    let big: Vec<u8> = (0..9_000_000u32).map(|n| (n % 251) as u8).collect();
    let files = vec![
        opened(&dir, "IMG_0042.jpg", &big),
        opened(&dir, "IMG_0042.jpg", b"another photo, same name"),
        opened(&dir, "notes/../plan.txt", b"plan"),
    ];
    // Gone from disk: only the open handles can still read them.
    for entry in fs::read_dir(dir.path()).unwrap() {
        fs::remove_file(entry.unwrap().path()).unwrap();
    }

    let prepared = prepare_opened_with_id(files, ID).unwrap();
    let names: Vec<_> = prepared
        .manifest()
        .items()
        .iter()
        .map(|item| item.path.to_string())
        .collect();
    assert_eq!(names[..2], ["IMG_0042.jpg", "IMG_0042 (2).jpg"]);
    assert!(
        !names[2].contains('/'),
        "a name never makes folders: {}",
        names[2]
    );

    let dest = dir.path().join("inbox");
    fs::create_dir(&dest).unwrap();
    send_all(prepared, &dest);
    assert_eq!(fs::read(dest.join("IMG_0042.jpg")).unwrap(), big);
    assert_eq!(
        fs::read(dest.join("IMG_0042 (2).jpg")).unwrap(),
        b"another photo, same name"
    );
}

#[test]
fn a_file_that_grows_after_the_offer_is_caught() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("log.txt");
    fs::write(&path, b"first line\n").unwrap();
    let file = File::open(&path).unwrap();
    let prepared = prepare_opened_with_id(
        vec![OpenedFile {
            name: "log.txt".into(),
            file,
        }],
        ID,
    )
    .unwrap();
    let source = prepared.source(0).unwrap().clone();
    source.ensure_unchanged().unwrap();

    let mut writer = OpenOptions::new().append(true).open(&path).unwrap();
    writer.write_all(b"second line\n").unwrap();
    assert!(matches!(
        source.ensure_unchanged(),
        Err(BeamError::SourceChanged { .. })
    ));
}

#[test]
fn folders_and_empty_selections_are_refused() {
    let dir = TempDir::new().unwrap();
    let folder = OpenedFile {
        name: "album".into(),
        file: File::open(dir.path()).unwrap(),
    };
    for files in [vec![folder], Vec::new()] {
        let result = prepare_opened_with_id(files, ID);
        assert!(matches!(result, Err(BeamError::Selection(_))), "{result:?}");
    }
}
