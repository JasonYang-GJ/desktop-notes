#![cfg(windows)]

use std::{fs, path::PathBuf, time::SystemTime};

use desktop_notes_core::{ErrorCode, KeyProtector};
use desktop_notes_windows::DpapiKeyring;

fn isolated_path() -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir()
        .join(format!("desktop-notes-b01-{}-{nonce}", std::process::id()))
        .join("keyring.json")
}

#[test]
fn current_user_keyring_round_trips_and_corruption_never_regenerates() {
    let path = isolated_path();
    let keyring = DpapiKeyring::new(path.clone());
    assert!(keyring.load_key().unwrap().is_none());

    let created = keyring.create_and_store_key().unwrap();
    let loaded = keyring.load_key().unwrap().unwrap();
    assert_eq!(loaded.as_bytes(), created.as_bytes());
    let backup_wrapped = keyring.export_wrapped_root().unwrap();
    let recovered_from_backup_blob = DpapiKeyring::unprotect_backup_root(&backup_wrapped).unwrap();
    assert_eq!(recovered_from_backup_blob.as_bytes(), created.as_bytes());

    let mut corrupted = fs::read(&path).unwrap();
    let middle = corrupted.len() / 2;
    corrupted[middle] ^= 1;
    fs::write(&path, &corrupted).unwrap();

    let error = keyring.load_key().unwrap_err();
    assert_eq!(error.code(), ErrorCode::KeyUnavailable);
    assert_eq!(fs::read(&path).unwrap(), corrupted);

    fs::remove_dir_all(path.parent().unwrap()).unwrap();
}
