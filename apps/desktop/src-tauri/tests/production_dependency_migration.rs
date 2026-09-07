#![cfg(windows)]

use std::{
    env, fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU32, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64_STANDARD};
use desktop_notes_core::{
    AssetIdGenerator, AssetStore, AssignTagRequest, Clock, CreateNoteRequest, EncryptedStore,
    FoundationError, ImageAssetService, ImageInput, ImageInputSource, ImageSourceFormat,
    ImportImageRequest, KeyDeriver, KeyProtector, Note, NoteIdGenerator, NoteRepository,
    NoteService, OrganizationRepository, OrganizationService, SearchRepository, SearchService,
    SetPinnedRequest, ShortcutSettingsStore, ShortcutSpec, Tag, extract_image_asset_occurrences,
};
use desktop_notes_infra::{
    AutomaticBackupManager, BackupClassification, BackupRunOutcome, BackupTrigger,
    EncryptedAssetFiles, HkdfKeyDeriver, SqlCipherStore,
};
use desktop_notes_windows::DpapiKeyring;
use serde::Serialize;
use serde_json::json;

const MIGRATION_ROOT_ENV: &str = "DESKTOP_NOTES_PRODUCTION_MIGRATION_ROOT";
const COMPARE_ROOT_ENV: &str = "DESKTOP_NOTES_PRODUCTION_MIGRATION_COMPARE_ROOT";
const WIN10_ROOT_ENV: &str = "DESKTOP_NOTES_PRODUCTION_WIN10_ROOT";
const WIN10_NOTE_ID: &str = "41800000-0000-4000-8000-000000000001";
const WIN10_ASSET_ID: &str = "41800000-0000-4000-8000-000000000002";
const WIN10_TAG_ID: &str = "41800000-0000-4000-8000-000000000003";
const WIN10_IMPORT_ID: &str = "41800000-0000-4000-8000-000000000004";
const WIN10_TITLE: &str = "Win10 production dependency migration";
const WIN10_BODY: &str = "Win10 真实产品数据 searchable body";
const WIN10_TAG: &str = "Win10迁移";

#[derive(Debug, Eq, PartialEq)]
struct LogicalSnapshot {
    notes: Vec<Note>,
    tags: Vec<Tag>,
    note_tags: Vec<(String, Vec<Tag>)>,
    recent_ids: Vec<String>,
    tracked_assets: Vec<String>,
    window_preferences: Option<String>,
    quick_capture_shortcut: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct MigrationEvidence {
    status: &'static str,
    database_state: &'static str,
    schema_version: u32,
    cipher_version: String,
    sqlite_version: &'static str,
    note_count: usize,
    stable_note_ids_preserved: bool,
    rich_text_records: usize,
    tag_count: usize,
    tagged_note_count: usize,
    pinned_note_count: usize,
    recent_count: usize,
    search_title_body_tag: bool,
    encrypted_assets_read: usize,
    quick_capture_shortcut_preserved: bool,
    automatic_backups_validated: usize,
    isolated_restore_probe: bool,
    window_preferences_preserved: bool,
    same_user_dpapi: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Win10CompatibilityEvidence {
    status: &'static str,
    database_state: &'static str,
    schema_version: u32,
    cipher_version: String,
    note_reopened: bool,
    rich_text_reopened: bool,
    tag_reopened: bool,
    pinned_reopened: bool,
    search_title_body_tag: bool,
    encrypted_image_reopened: bool,
    quick_capture_reopened: bool,
    window_preferences_reopened: bool,
    automatic_backup_validated: bool,
    same_user_dpapi: bool,
}

#[test]
#[ignore = "requires an explicitly supplied same-user B01-B10 product DataRoot clone"]
fn existing_b01_b10_product_data_reopens_on_the_frozen_production_runtime() {
    let root = required_root(MIGRATION_ROOT_ENV);
    let compare_root = required_root(COMPARE_ROOT_ENV);

    let opened = open_product_root(&root);
    assert!(
        !opened.status.newly_created,
        "must reopen an existing product DB"
    );
    assert_eq!(opened.status.schema_version, 7);
    assert_eq!(opened.status.cipher_version, "4.18.0 community");

    let migrated = logical_snapshot(&opened.store);
    let comparison = open_product_root(&compare_root);
    assert!(
        !comparison.status.newly_created,
        "comparison must be an existing product DB"
    );
    assert_eq!(comparison.status, opened.status);
    let original = logical_snapshot(&comparison.store);
    assert_eq!(
        migrated, original,
        "logical B01-B10 data changed during dependency migration"
    );
    assert!(
        !migrated.notes.is_empty(),
        "real migration fixture must contain Notes"
    );
    assert!(
        migrated
            .notes
            .iter()
            .all(|note| note.body_format == "tiptap-json")
    );
    assert!(migrated.notes.iter().any(|note| note.is_pinned));
    assert!(!migrated.tags.is_empty());
    assert!(migrated.note_tags.iter().any(|(_, tags)| !tags.is_empty()));
    assert!(!migrated.recent_ids.is_empty());
    assert!(migrated.window_preferences.is_some());
    assert!(migrated.quick_capture_shortcut.is_some());

    let search_ok = verify_search_surfaces(&opened.store, &migrated);
    let asset_key = opened.deriver.derive_asset_key(&opened.root_key).unwrap();
    let files = EncryptedAssetFiles::new(root.clone(), asset_key);
    let assets_read = verify_images(&opened.store, &files, &migrated.notes);
    assert!(
        assets_read > 0,
        "real migration fixture must contain an encrypted image"
    );

    let backup_key = opened.deriver.derive_backup_key(&opened.root_key).unwrap();
    let wrapped_root = opened.keyring.export_wrapped_root().unwrap();
    let backups = AutomaticBackupManager::new(root.clone(), backup_key, wrapped_root).unwrap();
    let discoveries = backups.discover().unwrap();
    assert!(
        !discoveries.is_empty(),
        "real migration fixture must contain an automatic backup"
    );
    assert!(
        discoveries
            .iter()
            .all(|entry| entry.classification == BackupClassification::Valid)
    );
    let validated_count = discoveries
        .iter()
        .filter(|entry| entry.validated.is_some())
        .count();
    assert_eq!(validated_count, discoveries.len());

    let newest = discoveries.last().unwrap();
    let restore_root = isolated_restore_root();
    backups
        .extract_validated_to_isolated(&newest.path, &restore_root)
        .unwrap();
    let restored_store = SqlCipherStore::new(
        restore_root.join("data").join("desktop-notes.db"),
        env!("CARGO_PKG_VERSION"),
    );
    let restored_status = restored_store
        .open_or_initialize(&opened.database_key)
        .unwrap();
    assert!(!restored_status.newly_created);
    assert_eq!(restored_status.cipher_version, "4.18.0 community");
    let restored = logical_snapshot(&restored_store);
    assert!(!restored.notes.is_empty());
    assert!(!restored.tags.is_empty());
    let restored_asset_key = opened.deriver.derive_asset_key(&opened.root_key).unwrap();
    let restored_files = EncryptedAssetFiles::new(restore_root.clone(), restored_asset_key);
    assert!(verify_images(&restored_store, &restored_files, &restored.notes) > 0);
    drop(restored_files);
    drop(restored_store);
    fs::remove_dir_all(&restore_root).unwrap();

    let evidence = MigrationEvidence {
        status: "PASS",
        database_state: "reopened",
        schema_version: opened.status.schema_version,
        cipher_version: opened.status.cipher_version,
        sqlite_version: "3.53.4",
        note_count: migrated.notes.len(),
        stable_note_ids_preserved: true,
        rich_text_records: migrated.notes.len(),
        tag_count: migrated.tags.len(),
        tagged_note_count: migrated
            .note_tags
            .iter()
            .filter(|(_, tags)| !tags.is_empty())
            .count(),
        pinned_note_count: migrated.notes.iter().filter(|note| note.is_pinned).count(),
        recent_count: migrated.recent_ids.len(),
        search_title_body_tag: search_ok,
        encrypted_assets_read: assets_read,
        quick_capture_shortcut_preserved: true,
        automatic_backups_validated: validated_count,
        isolated_restore_probe: true,
        window_preferences_preserved: true,
        same_user_dpapi: true,
    };
    println!(
        "PRODUCTION_MIGRATION_EVIDENCE={}",
        serde_json::to_string(&evidence).unwrap()
    );
}

#[test]
#[ignore = "requires a Win10 product DataRoot first created by the frozen 4.14 product"]
fn win10_product_data_is_written_and_reopened_on_the_frozen_production_runtime() {
    let root = required_root(WIN10_ROOT_ENV);
    let opened = open_product_root(&root);
    assert!(
        !opened.status.newly_created,
        "the Win10 DataRoot must first be created by the frozen 4.14 product"
    );
    assert_eq!(opened.status.schema_version, 7);
    assert_eq!(opened.status.cipher_version, "4.18.0 community");

    let ids = Win10ProbeIds::default();
    let asset_files = EncryptedAssetFiles::new(
        root.clone(),
        opened.deriver.derive_asset_key(&opened.root_key).unwrap(),
    );
    let image_service = ImageAssetService::new(
        &opened.store,
        &asset_files,
        &ids,
        &FixedClock(4_102_358_399_000),
    );
    let imported = image_service
        .import_image(ImportImageRequest {
            client_import_id: WIN10_IMPORT_ID.to_owned(),
            input: ImageInput {
                source: ImageInputSource::EditorPaste,
                declared_format: ImageSourceFormat::Png,
                bytes: one_pixel_png(),
            },
        })
        .unwrap();
    assert_eq!(imported.asset_id, WIN10_ASSET_ID);

    let note = NoteService::new(&opened.store, &ids, &FixedClock(4_102_358_399_100))
        .create_note(CreateNoteRequest {
            note_date: "2099-12-31".to_owned(),
            title: WIN10_TITLE.to_owned(),
            body_format: "tiptap-json".to_owned(),
            body_schema_version: 1,
            body_json: json!({
                "type": "doc",
                "content": [
                    {
                        "type": "paragraph",
                        "content": [{ "type": "text", "text": WIN10_BODY }]
                    },
                    {
                        "type": "paragraph",
                        "content": [{
                            "type": "imageRef",
                            "attrs": { "asset_id": WIN10_ASSET_ID }
                        }]
                    }
                ]
            }),
        })
        .unwrap();
    assert_eq!(note.id, WIN10_NOTE_ID);

    let organization =
        OrganizationService::new(&opened.store, &ids, &FixedClock(4_102_358_399_200));
    let tag = organization.create_tag(WIN10_TAG).unwrap();
    assert_eq!(tag.id, WIN10_TAG_ID);
    let tagged = organization
        .assign_tag(AssignTagRequest {
            note_id: note.id.clone(),
            tag_id: tag.id.clone(),
            base_revision: note.revision,
            client_change_id: "41800000-0000-4000-8000-000000000005".to_owned(),
        })
        .unwrap();
    let pinned = organization
        .set_pinned(SetPinnedRequest {
            note_id: note.id.clone(),
            is_pinned: true,
            base_revision: tagged.revision,
            client_change_id: "41800000-0000-4000-8000-000000000006".to_owned(),
        })
        .unwrap();
    assert!(pinned.is_pinned);

    opened
        .store
        .save_quick_capture_shortcut(
            &ShortcutSpec::parse("Ctrl+Alt+M").unwrap(),
            4_102_358_399_300,
        )
        .unwrap();
    opened
        .store
        .save_window_preferences(
            r#"{"width":1180,"height":760,"maximized":false,"scaleFactor":1.25}"#,
            4_102_358_399_400,
        )
        .unwrap();

    let search = SearchService::new(&opened.store);
    assert_eq!(search.search("migration").unwrap()[0].note.id, note.id);
    assert_eq!(search.search("真实产品").unwrap()[0].note.id, note.id);
    assert_eq!(search.search(WIN10_TAG).unwrap()[0].note.id, note.id);
    assert_eq!(
        image_service
            .read_image(WIN10_NOTE_ID, WIN10_ASSET_ID)
            .unwrap()
            .bytes,
        imported.bytes
    );

    let backup_key = opened.deriver.derive_backup_key(&opened.root_key).unwrap();
    let wrapped_root = opened.keyring.export_wrapped_root().unwrap();
    let backups = AutomaticBackupManager::new(root.clone(), backup_key, wrapped_root).unwrap();
    let created = match backups
        .run_if_due(
            &opened.store,
            &opened.database_key,
            &asset_files,
            4_102_358_400_000,
            "2099-12-31",
            BackupTrigger::Startup,
        )
        .unwrap()
    {
        BackupRunOutcome::Created(created) => created,
        BackupRunOutcome::SkippedSameDay(_) => {
            panic!("the dedicated Win10 probe must create its own backup")
        }
    };
    let backup_path = backups.backup_directory().join(&created.storage_path);
    assert_eq!(backups.validate(&backup_path).unwrap(), created);
    assert!(created.asset_count >= 1);

    drop(backups);
    drop(opened);

    let reopened = open_product_root(&root);
    assert!(!reopened.status.newly_created);
    assert_eq!(reopened.status.cipher_version, "4.18.0 community");
    let note = reopened.store.get(WIN10_NOTE_ID).unwrap().unwrap();
    assert_eq!(note.title, WIN10_TITLE);
    assert_eq!(note.body_format, "tiptap-json");
    assert!(note.body_text.contains(WIN10_BODY));
    assert!(note.is_pinned);
    let tags = reopened.store.list_tags_for_note(WIN10_NOTE_ID).unwrap();
    assert_eq!(tags.len(), 1);
    assert_eq!(tags[0].name, WIN10_TAG);
    let search = SearchService::new(&reopened.store);
    for query in ["migration", "真实产品", WIN10_TAG] {
        assert!(
            search
                .search(query)
                .unwrap()
                .iter()
                .any(|hit| hit.note.id == WIN10_NOTE_ID)
        );
    }
    let reopened_files = EncryptedAssetFiles::new(
        root.clone(),
        reopened
            .deriver
            .derive_asset_key(&reopened.root_key)
            .unwrap(),
    );
    let reopened_image = ImageAssetService::new(
        &reopened.store,
        &reopened_files,
        &Win10ProbeIds::default(),
        &FixedClock(4_102_358_400_100),
    )
    .read_image(WIN10_NOTE_ID, WIN10_ASSET_ID)
    .unwrap();
    assert_eq!(reopened_image.bytes, imported.bytes);
    assert_eq!(
        reopened
            .store
            .load_quick_capture_shortcut()
            .unwrap()
            .as_deref(),
        Some("Ctrl+Alt+M")
    );
    assert!(reopened.store.load_window_preferences().unwrap().is_some());
    let reopened_backups = AutomaticBackupManager::new(
        root,
        reopened
            .deriver
            .derive_backup_key(&reopened.root_key)
            .unwrap(),
        reopened.keyring.export_wrapped_root().unwrap(),
    )
    .unwrap();
    assert!(reopened_backups.discover().unwrap().iter().any(|entry| {
        entry.classification == BackupClassification::Valid
            && entry
                .validated
                .as_ref()
                .is_some_and(|backup| backup.backup_id == created.backup_id)
    }));

    let evidence = Win10CompatibilityEvidence {
        status: "PASS",
        database_state: "created-by-4.14-product-and-reopened-by-4.18-product",
        schema_version: reopened.status.schema_version,
        cipher_version: reopened.status.cipher_version,
        note_reopened: true,
        rich_text_reopened: true,
        tag_reopened: true,
        pinned_reopened: true,
        search_title_body_tag: true,
        encrypted_image_reopened: true,
        quick_capture_reopened: true,
        window_preferences_reopened: true,
        automatic_backup_validated: true,
        same_user_dpapi: true,
    };
    println!(
        "PRODUCTION_WIN10_EVIDENCE={}",
        serde_json::to_string(&evidence).unwrap()
    );
}

#[derive(Default)]
struct Win10ProbeIds {
    note_ids: AtomicU32,
    asset_ids: AtomicU32,
}

impl NoteIdGenerator for Win10ProbeIds {
    fn new_note_id(&self) -> Result<String, FoundationError> {
        match self.note_ids.fetch_add(1, Ordering::SeqCst) {
            0 => Ok(WIN10_NOTE_ID.to_owned()),
            1 => Ok(WIN10_TAG_ID.to_owned()),
            _ => Err(FoundationError::validation_failed()),
        }
    }
}

impl AssetIdGenerator for Win10ProbeIds {
    fn new_asset_id(&self) -> Result<String, FoundationError> {
        match self.asset_ids.fetch_add(1, Ordering::SeqCst) {
            0 => Ok(WIN10_ASSET_ID.to_owned()),
            _ => Err(FoundationError::validation_failed()),
        }
    }
}

struct FixedClock(i64);

impl Clock for FixedClock {
    fn now_utc_ms(&self) -> Result<i64, FoundationError> {
        Ok(self.0)
    }
}

fn one_pixel_png() -> Vec<u8> {
    BASE64_STANDARD
        .decode("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=")
        .unwrap()
}

struct OpenedRoot {
    keyring: DpapiKeyring,
    root_key: desktop_notes_core::RootKey,
    database_key: desktop_notes_core::SecretKey,
    deriver: HkdfKeyDeriver,
    store: SqlCipherStore,
    status: desktop_notes_core::DatabaseStatus,
}

fn open_product_root(root: &Path) -> OpenedRoot {
    let keyring = DpapiKeyring::new(root.join("control").join("keyring.json"));
    let root_key = keyring
        .load_key()
        .unwrap()
        .expect("the existing product DataRoot must already have a DPAPI key");
    let deriver = HkdfKeyDeriver;
    let database_key = deriver.derive_database_key(&root_key).unwrap();
    let store = SqlCipherStore::new(
        root.join("data").join("desktop-notes.db"),
        env!("CARGO_PKG_VERSION"),
    );
    let status = store.open_or_initialize(&database_key).unwrap();
    OpenedRoot {
        keyring,
        root_key,
        database_key,
        deriver,
        store,
        status,
    }
}

fn logical_snapshot(store: &SqlCipherStore) -> LogicalSnapshot {
    let dates = store
        .count_by_date_range("2000-01-01", "2100-01-01")
        .unwrap();
    let mut notes = Vec::new();
    for date in dates {
        let summaries = store.list_for_date(&date.note_date).unwrap();
        assert_eq!(summaries.len() as u64, date.count);
        for summary in summaries {
            notes.push(store.get(&summary.id).unwrap().unwrap());
        }
    }
    notes.sort_by(|left, right| left.id.cmp(&right.id));

    let mut tags = store.list_tags().unwrap();
    tags.sort_by(|left, right| left.id.cmp(&right.id));
    let mut note_tags = notes
        .iter()
        .map(|note| {
            let mut assigned = store.list_tags_for_note(&note.id).unwrap();
            assigned.sort_by(|left, right| left.id.cmp(&right.id));
            (note.id.clone(), assigned)
        })
        .collect::<Vec<_>>();
    note_tags.sort_by(|left, right| left.0.cmp(&right.0));
    let recent_ids = store
        .list_recent(100)
        .unwrap()
        .into_iter()
        .map(|note| note.id)
        .collect();
    let mut tracked_assets = store.tracked_storage_relpaths().unwrap();
    tracked_assets.sort();

    LogicalSnapshot {
        notes,
        tags,
        note_tags,
        recent_ids,
        tracked_assets,
        window_preferences: store.load_window_preferences().unwrap(),
        quick_capture_shortcut: store.load_quick_capture_shortcut().unwrap(),
    }
}

fn verify_search_surfaces(store: &SqlCipherStore, snapshot: &LogicalSnapshot) -> bool {
    let title_note = snapshot
        .notes
        .iter()
        .find(|note| note.title.chars().count() >= 3)
        .unwrap();
    let title_query = first_chars(&title_note.title, 3);
    assert!(
        store
            .search(&title_query, 100)
            .unwrap()
            .iter()
            .any(|hit| hit.note.id == title_note.id)
    );

    let body_note = snapshot
        .notes
        .iter()
        .find(|note| note.body_text.chars().count() >= 3)
        .unwrap();
    let body_query = first_chars(&body_note.body_text, 3);
    assert!(
        store
            .search(&body_query, 100)
            .unwrap()
            .iter()
            .any(|hit| hit.note.id == body_note.id)
    );

    let (tagged_note_id, assigned_tags) = snapshot
        .note_tags
        .iter()
        .find(|(_, tags)| !tags.is_empty())
        .unwrap();
    let tag_query = first_chars(
        &assigned_tags[0].name,
        3.min(assigned_tags[0].name.chars().count()),
    );
    assert!(
        store
            .search(&tag_query, 100)
            .unwrap()
            .iter()
            .any(|hit| hit.note.id == *tagged_note_id)
    );
    true
}

fn verify_images(store: &SqlCipherStore, files: &EncryptedAssetFiles, notes: &[Note]) -> usize {
    let mut read = 0;
    for note in notes {
        for asset_id in extract_image_asset_occurrences(&note.body_json).unwrap() {
            let asset = store
                .get_ready_asset_for_note(&note.id, &asset_id)
                .unwrap()
                .unwrap();
            let plaintext = files
                .read_image(&asset.asset_id, &asset.storage_relpath, &asset.sha256_plain)
                .unwrap();
            assert!(!plaintext.is_empty());
            read += 1;
        }
    }
    read
}

fn first_chars(value: &str, count: usize) -> String {
    value.chars().take(count).collect()
}

fn required_root(name: &str) -> PathBuf {
    let value = env::var_os(name).unwrap_or_else(|| panic!("{name} is required"));
    let root = PathBuf::from(value);
    assert!(
        root.is_absolute() && root.is_dir(),
        "{name} must be an existing absolute directory"
    );
    root
}

fn isolated_restore_root() -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    env::temp_dir().join(format!(
        "desktop-notes-production-restore-{}-{nonce}",
        std::process::id()
    ))
}
