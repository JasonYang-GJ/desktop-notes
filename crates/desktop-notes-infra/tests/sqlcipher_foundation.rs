use std::{fs, path::PathBuf, time::SystemTime};

use desktop_notes_core::{EncryptedStore, ErrorCode, KeyDeriver, RootKey, SecretKey};
use desktop_notes_infra::{HkdfKeyDeriver, Migration, MigrationRegistry, SqlCipherStore};
use rusqlite::{Connection, OpenFlags};

fn isolated_database() -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir()
        .join(format!(
            "desktop-notes-b01-db-{}-{nonce}",
            std::process::id()
        ))
        .join("desktop-notes.db")
}

fn key(byte: u8) -> SecretKey {
    SecretKey::from_bytes([byte; 32])
}

fn open_raw(path: &PathBuf, key_byte: u8) -> Connection {
    let connection = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .unwrap();
    connection
        .execute_batch(&format!(
            "PRAGMA key = \"x'{}'\";",
            format!("{key_byte:02x}").repeat(32)
        ))
        .unwrap();
    connection
}

#[test]
fn sqlcipher_first_run_reopens_and_wrong_key_does_not_replace_database() {
    let path = isolated_database();
    let first = SqlCipherStore::new(path.clone(), "0.1.0");
    let created = first.open_or_initialize(&key(0x31)).unwrap();
    assert!(created.newly_created);
    assert_eq!(created.schema_version, 7);
    assert_eq!(created.cipher_version, "4.18.0 community");
    drop(first);

    let encrypted = fs::read(&path).unwrap();
    assert!(!encrypted.starts_with(b"SQLite format 3"));
    assert!(
        !encrypted
            .windows(b"schema_migrations".len())
            .any(|window| window == b"schema_migrations")
    );

    let original = fs::read(&path).unwrap();
    let wrong = SqlCipherStore::new(path.clone(), "0.1.0");
    let error = wrong.open_or_initialize(&key(0x73)).unwrap_err();
    assert_eq!(error.code(), ErrorCode::DatabaseWrongKey);
    assert_eq!(fs::read(&path).unwrap(), original);
    drop(wrong);

    let reopened = SqlCipherStore::new(path.clone(), "0.1.0");
    let status = reopened.open_or_initialize(&key(0x31)).unwrap();
    assert!(!status.newly_created);
    assert_eq!(status.schema_version, 7);
    drop(reopened);

    fs::remove_dir_all(path.parent().unwrap()).unwrap();
}

#[test]
fn hkdf_database_key_is_deterministic_and_domain_separated() {
    let root = RootKey::from_bytes([0x55; 32]);
    let database = HkdfKeyDeriver.derive_database_key(&root).unwrap();
    let database_again = HkdfKeyDeriver.derive_database_key(&root).unwrap();

    assert_eq!(database.as_bytes(), database_again.as_bytes());
    assert_ne!(database.as_bytes(), root.as_bytes());
}

#[test]
fn failed_migration_rolls_back_and_current_registry_still_opens() {
    let path = isolated_database();
    let initial = SqlCipherStore::with_registry(path.clone(), "0.1.0", MigrationRegistry::b05());
    initial.open_or_initialize(&key(0x48)).unwrap();
    drop(initial);

    let failing_registry = MigrationRegistry::b05()
        .append(Migration::new(
            4,
            "intentional-test-failure",
            "CREATE TABLE must_rollback(id INTEGER); THIS IS NOT SQL;",
        ))
        .unwrap();
    let failing = SqlCipherStore::with_registry(path.clone(), "0.1.0", failing_registry);
    let error = failing.open_or_initialize(&key(0x48)).unwrap_err();
    assert_eq!(error.code(), ErrorCode::MigrationFailed);
    drop(failing);

    let current = SqlCipherStore::new(path.clone(), "0.1.0");
    let status = current.open_or_initialize(&key(0x48)).unwrap();
    assert_eq!(status.schema_version, 7);
    drop(current);

    fs::remove_dir_all(path.parent().unwrap()).unwrap();
}

#[test]
fn b08_database_migrates_forward_to_the_b09_backup_catalog() {
    let path = isolated_database();
    let b08 = SqlCipherStore::with_registry(path.clone(), "0.1.0", MigrationRegistry::b08());
    assert_eq!(
        b08.open_or_initialize(&key(0x61)).unwrap().schema_version,
        6
    );
    drop(b08);

    let b09 = SqlCipherStore::new(path.clone(), "0.1.0");
    assert_eq!(
        b09.open_or_initialize(&key(0x61)).unwrap().schema_version,
        7
    );
    drop(b09);

    let connection = open_raw(&path, 0x61);
    let table_count: i64 = connection
        .query_row(
            "SELECT count(*) FROM sqlite_master WHERE type = 'table' AND name = 'backup_catalog'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(table_count, 1);
    connection.close().unwrap();
    fs::remove_dir_all(path.parent().unwrap()).unwrap();
}

#[test]
fn newer_schema_is_refused_without_writing() {
    let path = isolated_database();
    let initial = SqlCipherStore::new(path.clone(), "0.1.0");
    initial.open_or_initialize(&key(0x29)).unwrap();
    drop(initial);

    let connection = open_raw(&path, 0x29);
    connection.pragma_update(None, "user_version", 99).unwrap();
    connection.close().unwrap();
    let before = fs::read(&path).unwrap();

    let future = SqlCipherStore::new(path.clone(), "0.1.0");
    let error = future.open_or_initialize(&key(0x29)).unwrap_err();
    assert_eq!(error.code(), ErrorCode::DatabaseSchemaTooNew);
    assert_eq!(fs::read(&path).unwrap(), before);
    drop(future);

    fs::remove_dir_all(path.parent().unwrap()).unwrap();
}

#[test]
fn encrypted_page_corruption_is_classified_and_never_replaced() {
    let path = isolated_database();
    let initial = SqlCipherStore::new(path.clone(), "0.1.0");
    initial.open_or_initialize(&key(0x62)).unwrap();
    drop(initial);

    let mut corrupted = fs::read(&path).unwrap();
    let last = corrupted.len() - 1;
    corrupted[last] ^= 1;
    fs::write(&path, &corrupted).unwrap();

    let damaged = SqlCipherStore::new(path.clone(), "0.1.0");
    let error = damaged.open_or_initialize(&key(0x62)).unwrap_err();
    assert_eq!(error.code(), ErrorCode::DatabaseCorrupted);
    assert_eq!(fs::read(&path).unwrap(), corrupted);
    drop(damaged);

    fs::remove_dir_all(path.parent().unwrap()).unwrap();
}

#[test]
fn a_directory_at_the_database_path_is_a_data_root_error() {
    let path = isolated_database();
    fs::create_dir_all(&path).unwrap();
    let store = SqlCipherStore::new(path.clone(), "0.1.0");

    let error = store.open_or_initialize(&key(0x17)).unwrap_err();

    assert_eq!(error.code(), ErrorCode::DataRootUnavailable);
    fs::remove_dir_all(path.parent().unwrap()).unwrap();
}
