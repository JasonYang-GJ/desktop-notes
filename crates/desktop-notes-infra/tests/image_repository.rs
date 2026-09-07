use std::{
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use desktop_notes_core::{
    ASSET_GC_GRACE_MS, AssetIdGenerator, AssignTagRequest, Clock, CreateNoteRequest,
    EncryptedStore, ErrorCode, FoundationError, ImageAssetService, ImageInput, ImageInputSource,
    ImageSourceFormat, ImportImageRequest, NoteIdGenerator, NoteService, OrganizationService,
    SearchService, SecretKey, SetPinnedRequest, UpdateNoteContentRequest,
    calculate_content_hash_hex,
};
use desktop_notes_infra::{EncryptedAssetFiles, MigrationRegistry, SqlCipherStore};
use image::{ExtendedColorType, ImageEncoder, codecs::png::PngEncoder};
use serde_json::{Value, json};

const NOTE_ID: &str = "70000000-0000-4000-8000-000000000001";
const ASSET_ID: &str = "70000000-0000-4000-8000-000000000002";
const IMPORT_ID: &str = "70000000-0000-4000-8000-000000000003";

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

#[test]
fn encrypted_image_is_committed_with_note_restarts_and_is_delayed_for_gc() {
    let root = isolated_root("product-chain");
    let path = root.join("data").join("desktop-notes.db");
    let store = SqlCipherStore::new(path.clone(), "0.1.0");
    assert_eq!(store.open_or_initialize(&key()).unwrap().schema_version, 7);
    create_note(&store, 1_000);

    let files = EncryptedAssetFiles::new(root.clone(), SecretKey::from_bytes([0x82; 32]));
    let importer = ImageAssetService::new(&store, &files, &FixedIds, &FixedClock(1_100));
    let imported = importer
        .import_image(import_request(encode_png(37, 23)))
        .unwrap();
    assert_eq!(imported.asset_id, ASSET_ID);
    assert_eq!((imported.pixel_width, imported.pixel_height), (37, 23));
    assert_eq!(
        importer.read_image(NOTE_ID, ASSET_ID).unwrap_err().code(),
        ErrorCode::AssetNotFound
    );

    let retry = importer
        .import_image(import_request(vec![0xff; 32]))
        .unwrap();
    assert_eq!(retry.asset_id, imported.asset_id);
    assert_eq!(retry.bytes, imported.bytes);

    let with_two_occurrences = image_document(ASSET_ID, true);
    let saved = update_note(&store, with_two_occurrences, 1, 1_200).unwrap();
    assert_eq!(saved.revision, 2);
    assert_eq!(
        importer.read_image(NOTE_ID, ASSET_ID).unwrap().bytes,
        imported.bytes
    );
    let encrypted_path = root.join(format!("assets/70/00/{ASSET_ID}.dnimg"));
    assert!(encrypted_path.is_file());
    let ciphertext = fs::read(&encrypted_path).unwrap();
    assert!(!contains(&ciphertext, &imported.bytes));
    drop(store);

    let reopened = SqlCipherStore::new(path, "0.1.0");
    assert_eq!(
        reopened.open_or_initialize(&key()).unwrap().schema_version,
        7
    );
    let reopened_files = EncryptedAssetFiles::new(root.clone(), SecretKey::from_bytes([0x82; 32]));
    let reopened_assets =
        ImageAssetService::new(&reopened, &reopened_files, &FixedIds, &FixedClock(1_250));
    assert_eq!(
        reopened_assets
            .repair_at_startup()
            .unwrap()
            .unknown_files_quarantined,
        0
    );
    assert_eq!(
        reopened_assets.read_image(NOTE_ID, ASSET_ID).unwrap().bytes,
        imported.bytes
    );

    let removed = update_note(&reopened, paragraph("searchable image removed"), 2, 1_300).unwrap();
    assert_eq!(removed.revision, 3);
    assert_eq!(
        reopened_assets
            .read_image(NOTE_ID, ASSET_ID)
            .unwrap_err()
            .code(),
        ErrorCode::AssetNotFound
    );
    assert!(
        encrypted_path.is_file(),
        "removal must be delayed, not aggressive"
    );
    let search = SearchService::new(&reopened);
    assert_eq!(
        search.search("searchable image").unwrap()[0].note.id,
        NOTE_ID
    );
    assert_eq!(
        reopened_assets
            .repair_at_startup()
            .unwrap()
            .due_assets_removed,
        0
    );
    let due_assets = ImageAssetService::new(
        &reopened,
        &reopened_files,
        &FixedIds,
        &FixedClock(1_300 + ASSET_GC_GRACE_MS),
    );
    assert_eq!(
        due_assets.repair_at_startup().unwrap().due_assets_removed,
        1
    );
    assert!(!encrypted_path.exists());
    drop(reopened);
    remove_root(&root);
}

#[test]
fn quick_capture_create_commits_the_staged_image_in_the_same_note_transaction() {
    let root = isolated_root("quick-create");
    let path = root.join("data").join("desktop-notes.db");
    let store = SqlCipherStore::new(path, "0.1.0");
    store.open_or_initialize(&key()).unwrap();
    let files = EncryptedAssetFiles::new(root.clone(), SecretKey::from_bytes([0x82; 32]));
    let assets = ImageAssetService::new(&store, &files, &FixedIds, &FixedClock(1_100));
    let imported = assets
        .import_image(import_request(encode_png(11, 9)))
        .unwrap();

    let created = NoteService::new(&store, &FixedIds, &FixedClock(1_200))
        .create_note(CreateNoteRequest {
            note_date: "2026-09-07".to_owned(),
            title: "Quick image".to_owned(),
            body_format: "tiptap-json".to_owned(),
            body_schema_version: 1,
            body_json: image_document(ASSET_ID, false),
        })
        .unwrap();

    assert_eq!(created.revision, 1);
    assert_eq!(created.note_date, "2026-09-07");
    assert_eq!(
        assets.read_image(NOTE_ID, ASSET_ID).unwrap().bytes,
        imported.bytes
    );
    assert!(
        root.join(format!("assets/70/00/{ASSET_ID}.dnimg"))
            .is_file()
    );
    drop(store);
    remove_root(&root);
}

#[test]
fn cancelled_quick_capture_removes_its_encrypted_staged_image() {
    let root = isolated_root("quick-cancel");
    let path = root.join("data").join("desktop-notes.db");
    let store = SqlCipherStore::new(path, "0.1.0");
    store.open_or_initialize(&key()).unwrap();
    let files = EncryptedAssetFiles::new(root.clone(), SecretKey::from_bytes([0x82; 32]));
    let assets = ImageAssetService::new(&store, &files, &FixedIds, &FixedClock(1_100));
    assets
        .import_image(import_request(encode_png(11, 9)))
        .unwrap();
    let encrypted_path = root.join(format!("assets/70/00/{ASSET_ID}.dnimg"));
    assert!(encrypted_path.is_file());

    assert!(assets.discard_import(IMPORT_ID).unwrap());
    assert!(!encrypted_path.exists());
    assert!(!assets.discard_import(IMPORT_ID).unwrap());
    assert!(
        NoteService::new(&store, &FixedIds, &FixedClock(1_200))
            .list_notes_for_date("2026-09-07")
            .unwrap()
            .is_empty()
    );
    drop(store);
    remove_root(&root);
}

#[test]
fn stale_or_incomplete_note_save_never_commits_a_broken_reference() {
    let root = isolated_root("rollback");
    let path = root.join("data").join("desktop-notes.db");
    let store = SqlCipherStore::new(path.clone(), "0.1.0");
    store.open_or_initialize(&key()).unwrap();
    create_note(&store, 2_000);
    let files = EncryptedAssetFiles::new(root.clone(), SecretKey::from_bytes([0x82; 32]));
    let assets = ImageAssetService::new(&store, &files, &FixedIds, &FixedClock(2_100));
    assets
        .import_image(import_request(encode_png(9, 7)))
        .unwrap();

    let stale = update_note(&store, image_document(ASSET_ID, false), 9, 2_200).unwrap_err();
    assert_eq!(stale.code(), ErrorCode::RevisionConflict);
    assert_eq!(
        NoteService::new(&store, &FixedIds, &FixedClock(2_250))
            .get_note(NOTE_ID)
            .unwrap()
            .revision,
        1
    );

    let unknown_id = "70000000-0000-4000-8000-000000000099";
    let invalid_transaction = json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [
                { "type": "imageRef", "attrs": { "asset_id": ASSET_ID } },
                { "type": "imageRef", "attrs": { "asset_id": unknown_id } }
            ]
        }]
    });
    assert_eq!(
        update_note(&store, invalid_transaction, 1, 2_300)
            .unwrap_err()
            .code(),
        ErrorCode::AssetNotFound
    );
    let note = NoteService::new(&store, &FixedIds, &FixedClock(2_350))
        .get_note(NOTE_ID)
        .unwrap();
    assert_eq!(note.revision, 1);
    assert!(!note.body_json.contains(ASSET_ID));
    drop(store);
    let partial_final_id = "70000000-0000-4000-8000-000000000098";
    let partial_final = root
        .join("assets")
        .join("70")
        .join("00")
        .join(format!("{partial_final_id}.dnimg"));
    fs::write(&partial_final, b"DNIMGv1\0partial final container").unwrap();
    fs::create_dir_all(root.join("staging")).unwrap();
    fs::write(
        root.join("staging")
            .join(format!("{ASSET_ID}-0123456789abcdef.part")),
        b"partial",
    )
    .unwrap();

    let reopened = SqlCipherStore::new(path, "0.1.0");
    reopened.open_or_initialize(&key()).unwrap();
    let reopened_files = EncryptedAssetFiles::new(root.clone(), SecretKey::from_bytes([0x82; 32]));
    let reopened_assets =
        ImageAssetService::new(&reopened, &reopened_files, &FixedIds, &FixedClock(2_400));
    let repair = reopened_assets.repair_at_startup().unwrap();
    assert_eq!(repair.staging_files_removed, 1);
    assert_eq!(repair.unknown_files_quarantined, 2);
    assert_eq!(count_files(&root.join("staging").join("quarantine")), 2);
    assert!(!partial_final.exists());
    assert_eq!(
        reopened_assets
            .read_image(NOTE_ID, ASSET_ID)
            .unwrap_err()
            .code(),
        ErrorCode::AssetNotFound
    );
    drop(reopened);
    remove_root(&root);
}

#[test]
fn b06_database_migrates_forward_without_recreation() {
    let root = isolated_root("migration");
    let path = root.join("data").join("desktop-notes.db");
    let b06 = SqlCipherStore::with_registry(path.clone(), "0.1.0", MigrationRegistry::b06());
    assert_eq!(b06.open_or_initialize(&key()).unwrap().schema_version, 4);
    create_note(&b06, 3_000);
    let rich_text = json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [
                { "type": "text", "marks": [{ "type": "bold" }], "text": "migration " },
                { "type": "text", "text": "rich searchable text" }
            ]
        }]
    });
    let saved = update_note(&b06, rich_text, 1, 3_100).unwrap();
    let pinned = OrganizationService::new(&b06, &FixedIds, &FixedClock(3_200))
        .set_pinned(SetPinnedRequest {
            note_id: NOTE_ID.to_owned(),
            is_pinned: true,
            base_revision: saved.revision,
            client_change_id: "70000000-0000-4000-8000-000000000031".to_owned(),
        })
        .unwrap();
    let before = OrganizationService::new(&b06, &FixedIds, &FixedClock(3_300))
        .assign_tag(AssignTagRequest {
            note_id: NOTE_ID.to_owned(),
            tag_id: "00000000-0000-4000-8000-000000000101".to_owned(),
            base_revision: pinned.revision,
            client_change_id: "70000000-0000-4000-8000-000000000032".to_owned(),
        })
        .unwrap();
    let before_read = NoteService::new(&b06, &FixedIds, &FixedClock(3_350))
        .get_note(NOTE_ID)
        .unwrap();
    assert_eq!(before, before_read);
    drop(b06);

    let b07 = SqlCipherStore::new(path, "0.1.0");
    assert_eq!(b07.open_or_initialize(&key()).unwrap().schema_version, 7);
    assert_eq!(
        NoteService::new(&b07, &FixedIds, &FixedClock(3_200))
            .get_note(NOTE_ID)
            .unwrap(),
        before
    );
    let notes_for_date = NoteService::new(&b07, &FixedIds, &FixedClock(3_400))
        .list_notes_for_date("2026-09-06")
        .unwrap();
    assert_eq!(notes_for_date.len(), 1);
    assert!(notes_for_date[0].is_pinned);
    let organization = OrganizationService::new(&b07, &FixedIds, &FixedClock(3_400));
    assert_eq!(organization.list_recent().unwrap()[0].id, NOTE_ID);
    assert_eq!(
        organization.list_tags_for_note(NOTE_ID).unwrap()[0].name,
        "Work"
    );
    assert_eq!(
        SearchService::new(&b07)
            .search("rich searchable")
            .unwrap()
            .len(),
        1
    );
    assert_eq!(SearchService::new(&b07).search("Work").unwrap().len(), 1);
    drop(b07);
    remove_root(&root);
}

fn key() -> SecretKey {
    SecretKey::from_bytes([0x81; 32])
}

fn create_note(store: &SqlCipherStore, time: i64) {
    NoteService::new(store, &FixedIds, &FixedClock(time))
        .create_note(CreateNoteRequest {
            note_date: "2026-09-06".to_owned(),
            title: "B07 encrypted image".to_owned(),
            body_format: "tiptap-json".to_owned(),
            body_schema_version: 1,
            body_json: paragraph("initial body"),
        })
        .unwrap();
}

fn update_note(
    store: &SqlCipherStore,
    body_json: Value,
    base_revision: u64,
    time: i64,
) -> Result<desktop_notes_core::Note, FoundationError> {
    let canonical = serde_json::to_string(&body_json).unwrap();
    NoteService::new(store, &FixedIds, &FixedClock(time)).update_note_content(
        UpdateNoteContentRequest {
            note_id: NOTE_ID.to_owned(),
            title: "B07 encrypted image".to_owned(),
            body_format: "tiptap-json".to_owned(),
            body_schema_version: 1,
            body_json,
            base_revision,
            client_change_id: format!("70000000-0000-4000-8000-{time:012}"),
            content_hash: calculate_content_hash_hex(
                "2026-09-06",
                "B07 encrypted image",
                &canonical,
            ),
        },
    )
}

fn import_request(bytes: Vec<u8>) -> ImportImageRequest {
    ImportImageRequest {
        client_import_id: IMPORT_ID.to_owned(),
        input: ImageInput {
            source: ImageInputSource::ClipboardScreenshot,
            declared_format: ImageSourceFormat::Png,
            bytes,
        },
    }
}

fn paragraph(text: &str) -> Value {
    json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [{ "type": "text", "text": text }]
        }]
    })
}

fn image_document(asset_id: &str, duplicate: bool) -> Value {
    let mut content = vec![
        json!({ "type": "text", "text": "before " }),
        json!({ "type": "imageRef", "attrs": { "asset_id": asset_id, "display_width": 320 } }),
        json!({ "type": "text", "text": " after" }),
    ];
    if duplicate {
        content.push(json!({ "type": "imageRef", "attrs": { "asset_id": asset_id } }));
    }
    json!({ "type": "doc", "content": [{ "type": "paragraph", "content": content }] })
}

fn encode_png(width: u32, height: u32) -> Vec<u8> {
    let pixels = vec![0x7f; (width * height * 4) as usize];
    let mut output = Vec::new();
    PngEncoder::new(&mut output)
        .write_image(&pixels, width, height, ExtendedColorType::Rgba8)
        .unwrap();
    output
}

fn isolated_root(label: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "desktop-notes-b07-{label}-{}-{nonce}",
        std::process::id()
    ));
    fs::create_dir_all(&root).unwrap();
    root
}

fn count_files(root: &Path) -> usize {
    fs::read_dir(root).unwrap().filter_map(Result::ok).count()
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

fn remove_root(root: &Path) {
    assert!(root.starts_with(std::env::temp_dir()));
    assert!(
        root.file_name()
            .and_then(|value| value.to_str())
            .is_some_and(|value| value.starts_with("desktop-notes-b07-"))
    );
    fs::remove_dir_all(root).unwrap();
}
