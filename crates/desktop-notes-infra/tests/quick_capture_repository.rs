use std::{
    fs,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

use desktop_notes_core::{EncryptedStore, SecretKey, ShortcutSettingsStore, ShortcutSpec};
use desktop_notes_infra::{MigrationRegistry, SqlCipherStore};

#[test]
fn b07_database_migrates_to_b08_settings_without_recreation() {
    let path = isolated_database();
    let b07 = SqlCipherStore::with_registry(path.clone(), "0.1.0", MigrationRegistry::b07());
    assert_eq!(b07.open_or_initialize(&key()).unwrap().schema_version, 5);
    drop(b07);

    let b08 = SqlCipherStore::new(path.clone(), "0.1.0");
    assert_eq!(b08.open_or_initialize(&key()).unwrap().schema_version, 7);
    assert_eq!(
        b08.load_quick_capture_shortcut().unwrap().as_deref(),
        Some("Ctrl+Alt+N")
    );
    b08.save_quick_capture_shortcut(
        &ShortcutSpec::parse("Ctrl+Alt+M").unwrap(),
        1_788_000_000_000,
    )
    .unwrap();
    drop(b08);

    let reopened = SqlCipherStore::new(path.clone(), "0.1.0");
    reopened.open_or_initialize(&key()).unwrap();
    assert_eq!(
        reopened.load_quick_capture_shortcut().unwrap().as_deref(),
        Some("Ctrl+Alt+M")
    );
    drop(reopened);
    fs::remove_dir_all(path.parent().unwrap()).unwrap();
}

fn key() -> SecretKey {
    SecretKey::from_bytes([0x91; 32])
}

fn isolated_database() -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "desktop-notes-b08-settings-{}-{nonce}",
        std::process::id()
    ));
    fs::create_dir_all(&root).unwrap();
    root.join("desktop-notes.db")
}
