use std::{
    fs::{self, OpenOptions},
    os::windows::fs::OpenOptionsExt,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        Arc, Barrier,
        atomic::{AtomicU32, Ordering},
    },
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use desktop_notes_core::{
    AssetIdGenerator, Clock, CreateNoteRequest, EncryptedStore, FoundationError, ImageAssetService,
    ImageInput, ImageInputSource, ImageSourceFormat, ImportImageRequest, NoteIdGenerator,
    NoteService, SecretKey,
};
use desktop_notes_infra::{
    AutomaticBackupManager, BackupClassification, BackupHealthState, BackupRunOutcome,
    BackupTrigger, EncryptedAssetFiles, SqlCipherStore,
};
use image::{ExtendedColorType, ImageEncoder, codecs::png::PngEncoder};
use rusqlite::{Connection, OpenFlags};
use serde_json::json;

const NOTE_ID: &str = "90000000-0000-4000-8000-000000000001";
const ASSET_ID: &str = "90000000-0000-4000-8000-000000000002";
const TITLE_MARKER: &str = "B09 synthetic backup title";
const BODY_MARKER: &str = "B09 synthetic backup body";

struct FixedIds;

impl NoteIdGenerator for FixedIds {
    fn new_note_id(&self) -> Result<String, FoundationError> {
        Ok(NOTE_ID.to_owned())
    }
}

impl AssetIdGenerator for FixedIds {
    fn new_asset_id(&self) -> Result<String, FoundationError> {
        Ok(ASSET_ID.to_owned())
    }
}

struct FixedClock(i64);

impl Clock for FixedClock {
    fn now_utc_ms(&self) -> Result<i64, FoundationError> {
        Ok(self.0)
    }
}

struct SequenceIds(AtomicU32);

impl NoteIdGenerator for SequenceIds {
    fn new_note_id(&self) -> Result<String, FoundationError> {
        let next = self.0.fetch_add(1, Ordering::SeqCst);
        Ok(format!("{next:08x}-0000-4000-8000-000000000001"))
    }
}

#[test]
fn automatic_backup_is_private_unique_discoverable_and_restorable() {
    let root = isolated_root("roundtrip");
    let database = SqlCipherStore::new(root.join("data/desktop-notes.db"), "0.1.0");
    assert_eq!(
        database
            .open_or_initialize(&database_key())
            .unwrap()
            .schema_version,
        7
    );
    let asset_files = EncryptedAssetFiles::new(root.clone(), asset_key());
    let expected_image = create_note_with_image(&database, &asset_files);
    let manager = backup_manager(root.clone());

    let created = match manager
        .run_if_due(
            &database,
            &database_key(),
            &asset_files,
            1_789_750_800_000,
            "2026-09-07",
            BackupTrigger::Startup,
        )
        .unwrap()
    {
        BackupRunOutcome::Created(created) => created,
        BackupRunOutcome::SkippedSameDay(_) => panic!("first run must create a generation"),
    };
    let package = manager.backup_directory().join(&created.storage_path);
    assert!(package.is_file());
    assert!(created.package_size > 0);
    assert_eq!(created.source_schema_version, 7);
    assert_eq!(created.asset_count, 1);

    let package_bytes = fs::read(&package).unwrap();
    for marker in [
        TITLE_MARKER.as_bytes(),
        BODY_MARKER.as_bytes(),
        expected_image.as_slice(),
        b"SQLite format 3".as_slice(),
    ] {
        assert!(!contains(&package_bytes, marker));
    }

    let validated = manager.validate(&package).unwrap();
    assert_eq!(validated, created);
    let discoveries = manager.discover().unwrap();
    assert_eq!(discoveries.len(), 1);
    assert_eq!(discoveries[0].classification, BackupClassification::Valid);
    assert_eq!(
        manager.refresh_health().unwrap().state,
        BackupHealthState::Healthy
    );
    assert_eq!(catalog_valid_count(database.path()), 1);

    let before = fs::read(&package).unwrap();
    let skipped = manager
        .run_if_due(
            &database,
            &database_key(),
            &asset_files,
            1_789_751_000_000,
            "2026-09-07",
            BackupTrigger::Interval,
        )
        .unwrap();
    assert!(matches!(skipped, BackupRunOutcome::SkippedSameDay(_)));
    assert_eq!(fs::read(&package).unwrap(), before);
    assert_eq!(manager.discover().unwrap().len(), 1);

    let isolated = isolated_root("restored");
    fs::remove_dir_all(&isolated).unwrap();
    manager
        .extract_validated_to_isolated(&package, &isolated)
        .unwrap();
    let restored_database = SqlCipherStore::new(isolated.join("data/desktop-notes.db"), "0.1.0");
    restored_database
        .open_or_initialize(&database_key())
        .unwrap();
    let restored_note = NoteService::new(&restored_database, &FixedIds, &FixedClock(0))
        .get_note(NOTE_ID)
        .unwrap();
    assert_eq!(restored_note.title, TITLE_MARKER);
    assert_eq!(restored_note.body_text.trim_end(), BODY_MARKER);
    let restored_files = EncryptedAssetFiles::new(isolated.clone(), asset_key());
    let restored_image = ImageAssetService::new(
        &restored_database,
        &restored_files,
        &FixedIds,
        &FixedClock(0),
    )
    .read_image(NOTE_ID, ASSET_ID)
    .unwrap();
    assert_eq!(restored_image.bytes, expected_image);

    drop(restored_database);
    drop(database);
    remove_root(&isolated);
    remove_root(&root);
}

#[test]
fn retention_counts_only_valid_generations_and_keeps_the_latest_thirty() {
    let root = isolated_root("retention");
    let database = SqlCipherStore::new(root.join("data/desktop-notes.db"), "0.1.0");
    database.open_or_initialize(&database_key()).unwrap();
    NoteService::new(&database, &FixedIds, &FixedClock(1_000))
        .create_note(CreateNoteRequest {
            note_date: "2026-08-01".to_owned(),
            title: TITLE_MARKER.to_owned(),
            body_format: "tiptap-json".to_owned(),
            body_schema_version: 1,
            body_json: paragraph(BODY_MARKER),
        })
        .unwrap();
    let assets = EncryptedAssetFiles::new(root.clone(), asset_key());
    let manager = backup_manager(root.clone());

    let mut first_id = None;
    for day in 1..=31 {
        let outcome = manager
            .run_if_due(
                &database,
                &database_key(),
                &assets,
                1_780_000_000_000 + i64::from(day) * 86_400_000,
                &format!("2026-08-{day:02}"),
                BackupTrigger::Interval,
            )
            .unwrap();
        if day == 1 {
            first_id = match outcome {
                BackupRunOutcome::Created(backup) => Some(backup.backup_id),
                BackupRunOutcome::SkippedSameDay(_) => unreachable!(),
            };
        }
    }

    let discoveries = manager.discover().unwrap();
    assert_eq!(discoveries.len(), 30);
    assert!(
        discoveries
            .iter()
            .all(|entry| entry.classification == BackupClassification::Valid)
    );
    assert!(discoveries.iter().all(|entry| {
        entry
            .validated
            .as_ref()
            .map(|backup| backup.backup_id.as_str())
            != first_id.as_deref()
    }));
    assert_eq!(catalog_valid_count(database.path()), 30);

    drop(database);
    remove_root(&root);
}

#[test]
fn a_locked_authoritative_asset_fails_safely_and_preserves_the_last_valid_backup() {
    let root = isolated_root("locked-asset");
    let database = SqlCipherStore::new(root.join("data/desktop-notes.db"), "0.1.0");
    database.open_or_initialize(&database_key()).unwrap();
    let assets = EncryptedAssetFiles::new(root.clone(), asset_key());
    create_note_with_image(&database, &assets);
    let manager = backup_manager(root.clone());
    manager
        .run_if_due(
            &database,
            &database_key(),
            &assets,
            1_789_750_800_000,
            "2026-09-07",
            BackupTrigger::Startup,
        )
        .unwrap();
    assert_eq!(manager.discover().unwrap().len(), 1);

    let asset_path = root.join(format!("assets/90/00/{ASSET_ID}.dnimg"));
    let locked = OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(asset_path)
        .unwrap();
    let error = manager
        .run_if_due(
            &database,
            &database_key(),
            &assets,
            1_789_837_200_000,
            "2026-09-08",
            BackupTrigger::Interval,
        )
        .unwrap_err();
    assert!(matches!(
        error.code(),
        desktop_notes_core::ErrorCode::BackupFailed
            | desktop_notes_core::ErrorCode::DataRootUnavailable
    ));
    let failed_health = manager.health();
    assert_eq!(failed_health.state, BackupHealthState::Failed);
    assert_eq!(
        failed_health.last_success_local_day.as_deref(),
        Some("2026-09-07")
    );
    drop(locked);
    let after = manager.discover().unwrap();
    assert_eq!(after.len(), 1);
    assert_eq!(after[0].classification, BackupClassification::Valid);
    assert_eq!(
        NoteService::new(&database, &FixedIds, &FixedClock(0))
            .get_note(NOTE_ID)
            .unwrap()
            .title,
        TITLE_MARKER
    );

    drop(database);
    remove_root(&root);
}

#[test]
fn directory_permission_denial_preserves_active_data_and_the_last_valid_backup() {
    let root = isolated_root("permission-denied");
    let database = SqlCipherStore::new(root.join("data/desktop-notes.db"), "0.1.0");
    database.open_or_initialize(&database_key()).unwrap();
    NoteService::new(&database, &FixedIds, &FixedClock(1_000))
        .create_note(CreateNoteRequest {
            note_date: "2026-09-07".to_owned(),
            title: TITLE_MARKER.to_owned(),
            body_format: "tiptap-json".to_owned(),
            body_schema_version: 1,
            body_json: paragraph(BODY_MARKER),
        })
        .unwrap();
    let assets = EncryptedAssetFiles::new(root.clone(), asset_key());
    let manager = backup_manager(root.clone());
    manager
        .run_if_due(
            &database,
            &database_key(),
            &assets,
            1_789_750_800_000,
            "2026-09-07",
            BackupTrigger::Startup,
        )
        .unwrap();
    let before = manager.discover().unwrap();
    let valid_path = before[0].path.clone();
    let valid_bytes = fs::read(&valid_path).unwrap();

    let principal = String::from_utf8(Command::new("whoami.exe").output().unwrap().stdout)
        .unwrap()
        .trim()
        .to_owned();
    let backup_directory = manager.backup_directory();
    let deny = Command::new("icacls.exe")
        .arg(&backup_directory)
        .arg("/deny")
        .arg(format!("{principal}:(W)"))
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .unwrap();
    assert!(deny.success());
    let restore = AclRestore {
        path: backup_directory,
        principal,
    };

    let error = manager
        .run_if_due(
            &database,
            &database_key(),
            &assets,
            1_789_837_200_000,
            "2026-09-08",
            BackupTrigger::Interval,
        )
        .unwrap_err();
    assert_eq!(error.code(), desktop_notes_core::ErrorCode::BackupFailed);
    let failed_health = manager.health();
    assert_eq!(failed_health.state, BackupHealthState::Failed);
    assert_eq!(
        failed_health.last_success_local_day.as_deref(),
        Some("2026-09-07")
    );
    assert_eq!(
        NoteService::new(&database, &FixedIds, &FixedClock(0))
            .get_note(NOTE_ID)
            .unwrap()
            .title,
        TITLE_MARKER
    );
    drop(restore);
    assert_eq!(fs::read(&valid_path).unwrap(), valid_bytes);
    assert_eq!(manager.discover().unwrap().len(), 1);

    drop(database);
    remove_root(&root);
}

#[test]
fn concurrent_triggers_admit_exactly_one_writer() {
    let root = isolated_root("single-writer");
    let database = Arc::new(SqlCipherStore::new(
        root.join("data/desktop-notes.db"),
        "0.1.0",
    ));
    database.open_or_initialize(&database_key()).unwrap();
    let ids = SequenceIds(AtomicU32::new(1));
    let large_body = "concurrent-backup-marker-".repeat(8_000);
    for index in 0..24 {
        NoteService::new(database.as_ref(), &ids, &FixedClock(1_000 + index))
            .create_note(CreateNoteRequest {
                note_date: "2026-09-07".to_owned(),
                title: format!("Concurrent {index}"),
                body_format: "tiptap-json".to_owned(),
                body_schema_version: 1,
                body_json: paragraph(&large_body),
            })
            .unwrap();
    }
    let assets = Arc::new(EncryptedAssetFiles::new(root.clone(), asset_key()));
    let manager = Arc::new(backup_manager(root.clone()));
    let barrier = Arc::new(Barrier::new(3));
    let mut workers = Vec::new();
    for trigger in [BackupTrigger::Startup, BackupTrigger::Resumed] {
        let database = Arc::clone(&database);
        let assets = Arc::clone(&assets);
        let manager = Arc::clone(&manager);
        let barrier = Arc::clone(&barrier);
        workers.push(std::thread::spawn(move || {
            barrier.wait();
            manager.run_if_due(
                database.as_ref(),
                &database_key(),
                assets.as_ref(),
                1_789_750_800_000,
                "2026-09-07",
                trigger,
            )
        }));
    }
    barrier.wait();
    let results = workers
        .into_iter()
        .map(|worker| worker.join().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(
        results
            .iter()
            .filter(|result| matches!(result, Ok(BackupRunOutcome::Created(_))))
            .count(),
        1
    );
    assert_eq!(
        results
            .iter()
            .filter(|result| {
                result
                    .as_ref()
                    .is_err_and(|error| error.code() == desktop_notes_core::ErrorCode::BackupBusy)
            })
            .count(),
        1
    );
    assert_eq!(manager.discover().unwrap().len(), 1);

    drop(database);
    remove_root(&root);
}

#[test]
fn process_kill_while_streaming_never_creates_a_falsely_valid_generation() {
    let root = isolated_root("process-kill");
    let database = SqlCipherStore::new(root.join("data/desktop-notes.db"), "0.1.0");
    database.open_or_initialize(&database_key()).unwrap();
    NoteService::new(&database, &FixedIds, &FixedClock(1_000))
        .create_note(CreateNoteRequest {
            note_date: "2026-09-07".to_owned(),
            title: TITLE_MARKER.to_owned(),
            body_format: "tiptap-json".to_owned(),
            body_schema_version: 1,
            body_json: paragraph(BODY_MARKER),
        })
        .unwrap();
    let assets = EncryptedAssetFiles::new(root.clone(), asset_key());
    let manager = backup_manager(root.clone());
    let prior = match manager
        .run_if_due(
            &database,
            &database_key(),
            &assets,
            1_789_750_800_000,
            "2026-09-07",
            BackupTrigger::Startup,
        )
        .unwrap()
    {
        BackupRunOutcome::Created(backup) => backup,
        BackupRunOutcome::SkippedSameDay(_) => unreachable!(),
    };
    let prior_path = manager.backup_directory().join(&prior.storage_path);
    let prior_bytes = fs::read(&prior_path).unwrap();
    drop(database);

    let mut child = Command::new(std::env::current_exe().unwrap())
        .arg("--exact")
        .arg("process_kill_worker")
        .arg("--nocapture")
        .env("DESKTOP_NOTES_S06_KILL_ROOT", &root)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(90);
    let partial_path = loop {
        let candidate = fs::read_dir(manager.backup_directory())
            .unwrap()
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .find(|path| path != &prior_path);
        if let Some(path) = candidate {
            break path;
        }
        assert!(
            Instant::now() < deadline,
            "child never opened a final package"
        );
        if child.try_wait().unwrap().is_some() {
            panic!("child completed before the kill point was observable");
        }
        thread::sleep(Duration::from_millis(1));
    };
    child.kill().unwrap();
    let _ = child.wait();

    assert!(partial_path.is_file());
    let discoveries = manager.discover().unwrap();
    assert_eq!(
        discoveries
            .iter()
            .filter(|entry| entry.classification == BackupClassification::Valid)
            .count(),
        1
    );
    assert!(discoveries.iter().any(|entry| {
        entry.path == partial_path && entry.classification != BackupClassification::Valid
    }));
    let startup_health = manager.refresh_health().unwrap();
    assert_eq!(startup_health.state, BackupHealthState::Failed);
    assert_eq!(
        startup_health.last_success_local_day.as_deref(),
        Some("2026-09-07")
    );
    assert_eq!(fs::read(&prior_path).unwrap(), prior_bytes);

    let reopened = SqlCipherStore::new(root.join("data/desktop-notes.db"), "0.1.0");
    reopened.open_or_initialize(&database_key()).unwrap();
    assert_eq!(
        NoteService::new(&reopened, &FixedIds, &FixedClock(0))
            .get_note(NOTE_ID)
            .unwrap()
            .title,
        TITLE_MARKER
    );
    drop(reopened);
    remove_root(&root);
}

#[test]
fn process_kill_worker() {
    let Some(root) = std::env::var_os("DESKTOP_NOTES_S06_KILL_ROOT").map(PathBuf::from) else {
        return;
    };
    let database = SqlCipherStore::new(root.join("data/desktop-notes.db"), "0.1.0");
    database.open_or_initialize(&database_key()).unwrap();
    let ids = SequenceIds(AtomicU32::new(0xa000_0001));
    let body = "process-kill-stream-marker-".repeat(10_000);
    for index in 0..96 {
        NoteService::new(&database, &ids, &FixedClock(2_000 + index))
            .create_note(CreateNoteRequest {
                note_date: "2026-09-08".to_owned(),
                title: format!("Kill worker {index}"),
                body_format: "tiptap-json".to_owned(),
                body_schema_version: 1,
                body_json: paragraph(&body),
            })
            .unwrap();
    }
    let assets = EncryptedAssetFiles::new(root.clone(), asset_key());
    backup_manager(root)
        .run_if_due(
            &database,
            &database_key(),
            &assets,
            1_789_837_200_000,
            "2026-09-08",
            BackupTrigger::Interval,
        )
        .unwrap();
}

#[test]
fn discovery_distinguishes_truncated_tampered_and_unsupported_packages() {
    let source_root = isolated_root("classification-source");
    let database = SqlCipherStore::new(source_root.join("data/desktop-notes.db"), "0.1.0");
    database.open_or_initialize(&database_key()).unwrap();
    NoteService::new(&database, &FixedIds, &FixedClock(1_000))
        .create_note(CreateNoteRequest {
            note_date: "2026-09-07".to_owned(),
            title: TITLE_MARKER.to_owned(),
            body_format: "tiptap-json".to_owned(),
            body_schema_version: 1,
            body_json: paragraph(BODY_MARKER),
        })
        .unwrap();
    let assets = EncryptedAssetFiles::new(source_root.clone(), asset_key());
    let source_manager = backup_manager(source_root.clone());
    let created = match source_manager
        .run_if_due(
            &database,
            &database_key(),
            &assets,
            1_789_750_800_000,
            "2026-09-07",
            BackupTrigger::Startup,
        )
        .unwrap()
    {
        BackupRunOutcome::Created(created) => created,
        BackupRunOutcome::SkippedSameDay(_) => unreachable!(),
    };
    let source = source_manager
        .backup_directory()
        .join(&created.storage_path);
    let bytes = fs::read(&source).unwrap();

    assert_classification(
        "incomplete",
        &created.storage_path,
        &bytes[..bytes.len() - 24],
        BackupClassification::Incomplete,
    );

    let mut tampered = bytes.clone();
    let tamper_index = tampered.len() / 2;
    tampered[tamper_index] ^= 0x40;
    assert_classification(
        "tampered",
        &created.storage_path,
        &tampered,
        BackupClassification::Corrupt,
    );

    let mut unsupported = bytes;
    unsupported[8..10].copy_from_slice(&2_u16.to_le_bytes());
    assert_classification(
        "unsupported",
        &created.storage_path,
        &unsupported,
        BackupClassification::Unsupported,
    );

    drop(database);
    remove_root(&source_root);
}

fn create_note_with_image(database: &SqlCipherStore, files: &EncryptedAssetFiles) -> Vec<u8> {
    let original = encode_png(17, 13);
    let imported = ImageAssetService::new(database, files, &FixedIds, &FixedClock(1_000))
        .import_image(ImportImageRequest {
            client_import_id: "90000000-0000-4000-8000-000000000003".to_owned(),
            input: ImageInput {
                source: ImageInputSource::EditorPaste,
                declared_format: ImageSourceFormat::Png,
                bytes: original,
            },
        })
        .unwrap();
    NoteService::new(database, &FixedIds, &FixedClock(1_100))
        .create_note(CreateNoteRequest {
            note_date: "2026-09-07".to_owned(),
            title: TITLE_MARKER.to_owned(),
            body_format: "tiptap-json".to_owned(),
            body_schema_version: 1,
            body_json: json!({
                "type": "doc",
                "content": [
                    {
                        "type": "paragraph",
                        "content": [{ "type": "text", "text": BODY_MARKER }]
                    },
                    {
                        "type": "paragraph",
                        "content": [{
                            "type": "imageRef",
                            "attrs": { "asset_id": ASSET_ID }
                        }]
                    }
                ]
            }),
        })
        .unwrap();
    imported.bytes
}

fn assert_classification(
    label: &str,
    filename: &str,
    bytes: &[u8],
    expected: BackupClassification,
) {
    let root = isolated_root(label);
    let directory = root.join("backups/automatic");
    fs::create_dir_all(&directory).unwrap();
    fs::write(directory.join(filename), bytes).unwrap();
    let manager = backup_manager(root.clone());
    let discoveries = manager.discover().unwrap();
    assert_eq!(discoveries.len(), 1);
    assert_eq!(discoveries[0].classification, expected);
    assert!(discoveries[0].validated.is_none());
    assert_eq!(
        manager.refresh_health().unwrap().state,
        BackupHealthState::Failed
    );
    remove_root(&root);
}

fn paragraph(text: &str) -> serde_json::Value {
    json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [{ "type": "text", "text": text }]
        }]
    })
}

fn encode_png(width: u32, height: u32) -> Vec<u8> {
    let pixels = vec![0x91_u8; (width * height * 4) as usize];
    let mut encoded = Vec::new();
    PngEncoder::new(&mut encoded)
        .write_image(&pixels, width, height, ExtendedColorType::Rgba8)
        .unwrap();
    encoded
}

fn backup_manager(root: PathBuf) -> AutomaticBackupManager {
    AutomaticBackupManager::new(root, backup_key(), vec![0x44; 96]).unwrap()
}

fn database_key() -> SecretKey {
    SecretKey::from_bytes([0x81; 32])
}

fn asset_key() -> SecretKey {
    SecretKey::from_bytes([0x82; 32])
}

fn backup_key() -> SecretKey {
    SecretKey::from_bytes([0x83; 32])
}

fn catalog_valid_count(path: &Path) -> i64 {
    let connection = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .unwrap();
    connection
        .execute_batch(&format!("PRAGMA key = \"x'{}'\";", "81".repeat(32)))
        .unwrap();
    connection
        .query_row(
            "SELECT count(*) FROM backup_catalog WHERE status = 'valid'",
            [],
            |row| row.get(0),
        )
        .unwrap()
}

fn isolated_root(label: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "desktop-notes-b09-{label}-{}-{nonce}",
        std::process::id()
    ));
    fs::create_dir_all(&root).unwrap();
    root
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    !needle.is_empty()
        && haystack
            .windows(needle.len())
            .any(|window| window == needle)
}

fn remove_root(root: &Path) {
    if root.exists() {
        fs::remove_dir_all(root).unwrap();
    }
}

struct AclRestore {
    path: PathBuf,
    principal: String,
}

impl Drop for AclRestore {
    fn drop(&mut self) {
        let _ = Command::new("icacls.exe")
            .arg(&self.path)
            .arg("/remove:d")
            .arg(&self.principal)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
}
