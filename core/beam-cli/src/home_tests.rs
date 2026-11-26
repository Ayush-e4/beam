// Test code reports a broken setup by panicking.
#![allow(clippy::unwrap_used)]

use std::os::unix::fs::PermissionsExt;

use tempfile::TempDir;

use super::*;

fn paired(id: &str, name: &str) -> PairedDevice {
    PairedDevice {
        public_key: vec![7; 32],
        device_id: id.into(),
        name: name.into(),
        cipher: SessionCipher::ChaChaPoly,
        verification_code: "123456".into(),
        discovery_key: vec![1; 32],
    }
}

#[test]
fn the_identity_is_created_once_and_kept_private() {
    let dir = TempDir::new().unwrap();
    let home = Home::at(dir.path());
    let first = home.identity().unwrap();
    let again = home.identity().unwrap();
    assert_eq!(first.public_key(), again.public_key());
    let mode = fs::metadata(home.file("identity.key"))
        .unwrap()
        .permissions()
        .mode();
    assert_eq!(mode & 0o777, 0o600);
}

#[test]
fn trusted_devices_survive_and_pairing_again_replaces() {
    let dir = TempDir::new().unwrap();
    let home = Home::at(dir.path());
    home.trust(&paired("aa", "Pixel")).unwrap();
    home.trust(&paired("bb", "Galaxy\tS21")).unwrap();
    home.trust(&paired("aa", "Pixel 9")).unwrap();
    let names: Vec<String> = Home::at(dir.path())
        .trusted()
        .unwrap()
        .into_iter()
        .map(|device| device.name)
        .collect();
    assert_eq!(names, ["Galaxy S21", "Pixel 9"]);
    let found = home.find("Galaxy S21").unwrap();
    assert_eq!(found.public_key, vec![7; 32]);
    assert_eq!(found.cipher, SessionCipher::ChaChaPoly);
    assert_eq!(home.find("aa").unwrap().name, "Pixel 9");
    assert!(home.find("nobody").is_err());
}

#[test]
fn a_damaged_list_is_an_error() {
    let dir = TempDir::new().unwrap();
    let home = Home::at(dir.path());
    fs::write(home.file("trusted.txt"), "aa\tquantum\t00\tX\n").unwrap();
    assert!(home.trusted().is_err());
}

#[test]
fn hex_round_trips() {
    assert_eq!(hex(&[0, 15, 255]), "000fff");
    assert_eq!(unhex("000fff"), Some(vec![0, 15, 255]));
    assert_eq!(unhex("0g"), None);
    assert_eq!(unhex("abc"), None);
}
