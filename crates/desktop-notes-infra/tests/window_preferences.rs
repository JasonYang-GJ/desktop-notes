use std::{
    fs,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

use desktop_notes_core::{EncryptedStore, SecretKey};
use desktop_notes_infra::SqlCipherStore;

#[test]
fn encrypted_window_preferences_survive_a_real_database_reopen() {
    let path = isolated_database();
    let payload = r#"{"version":1,"visualState":"expanded","theme":"dark","collapsed":null,"normal":null,"expanded":null}"#;

    let first = SqlCipherStore::new(path.clone(), "0.1.0");
    first.open_or_initialize(&key()).unwrap();
    first
        .save_window_preferences(payload, 1_789_000_000_000)
        .unwrap();
    drop(first);

    let reopened = SqlCipherStore::new(path.clone(), "0.1.0");
    reopened.open_or_initialize(&key()).unwrap();
    assert_eq!(
        reopened.load_window_preferences().unwrap().as_deref(),
        Some(payload)
    );
    drop(reopened);
    fs::remove_dir_all(path.parent().unwrap()).unwrap();
}

fn key() -> SecretKey {
    SecretKey::from_bytes([0xB1; 32])
}

fn isolated_database() -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "desktop-notes-b10-window-preferences-{}-{nonce}",
        std::process::id()
    ));
    fs::create_dir_all(&root).unwrap();
    root.join("desktop-notes.db")
}
