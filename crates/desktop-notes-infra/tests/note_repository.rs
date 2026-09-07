use std::{collections::HashMap, fs, path::PathBuf, sync::Mutex, time::SystemTime};

use desktop_notes_core::{
    CalendarService, ChangeNoteDateRequest, Clock, CreateNoteRequest, DateUndoRecord,
    DateUndoStore, DeleteNoteRequest, EncryptedStore, ErrorCode, FoundationError, NoteIdGenerator,
    NoteRepository, NoteService, SecretKey, UndoNoteDateChangeRequest, UpdateNoteContentRequest,
    calculate_content_hash_hex,
};
use desktop_notes_infra::SqlCipherStore;
use serde_json::json;

const NOTE_ID: &str = "8a02f56c-9677-4f30-9a30-f48e408b14ab";
const CHANGE_ID: &str = "cbb3e1b1-2bee-4c45-9b79-bdbb8f21e528";
const TITLE_MARKER: &str = "B02 Test Note";
const BODY_MARKER: &str = "B02 synthetic body marker";

struct FixedId;

impl NoteIdGenerator for FixedId {
    fn new_note_id(&self) -> Result<String, FoundationError> {
        Ok(NOTE_ID.to_owned())
    }
}

struct SequenceClock(Mutex<Vec<i64>>);

impl Clock for SequenceClock {
    fn now_utc_ms(&self) -> Result<i64, FoundationError> {
        Ok(self.0.lock().unwrap().remove(0))
    }
}

struct QueueIds(Mutex<Vec<String>>);

impl NoteIdGenerator for QueueIds {
    fn new_note_id(&self) -> Result<String, FoundationError> {
        Ok(self.0.lock().unwrap().remove(0))
    }
}

#[derive(Default)]
struct MemoryUndoStore(Mutex<HashMap<String, DateUndoRecord>>);

impl DateUndoStore for MemoryUndoStore {
    fn remember(&self, record: DateUndoRecord) {
        self.0
            .lock()
            .unwrap()
            .insert(record.token_id.clone(), record);
    }

    fn get(&self, token_id: &str) -> Option<DateUndoRecord> {
        self.0.lock().unwrap().get(token_id).cloned()
    }

    fn remove(&self, token_id: &str) {
        self.0.lock().unwrap().remove(token_id);
    }
}

fn isolated_database() -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir()
        .join(format!(
            "desktop-notes-b02-repository-{}-{nonce}",
            std::process::id()
        ))
        .join("desktop-notes.db")
}

fn key(byte: u8) -> SecretKey {
    SecretKey::from_bytes([byte; 32])
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

#[test]
fn encrypted_repository_persists_note_and_rolls_back_failed_update() {
    let path = isolated_database();
    let store = SqlCipherStore::new(path.clone(), "0.1.0");
    let status = store.open_or_initialize(&key(0x42)).unwrap();
    assert_eq!(status.schema_version, 7);

    let clock = SequenceClock(Mutex::new(vec![1_780_000_000_000, 1_780_000_000_750]));
    let service = NoteService::new(&store, &FixedId, &clock);
    let created = service
        .create_note(CreateNoteRequest {
            note_date: "2026-09-06".to_owned(),
            title: TITLE_MARKER.to_owned(),
            body_format: "tiptap-json".to_owned(),
            body_schema_version: 1,
            body_json: paragraph(BODY_MARKER),
        })
        .unwrap();
    assert_eq!(created.revision, 1);
    drop(store);

    let reopened = SqlCipherStore::new(path.clone(), "0.1.0");
    reopened.open_or_initialize(&key(0x42)).unwrap();
    let service = NoteService::new(&reopened, &FixedId, &clock);
    let loaded = service.get_note(NOTE_ID).unwrap();
    assert_eq!(loaded, created);
    assert_eq!(service.list_notes_for_date("2026-09-06").unwrap().len(), 1);

    let changed_body = paragraph("B02 updated synthetic body marker");
    let canonical_changed = serde_json::to_string(&changed_body).unwrap();
    let updated = service
        .update_note_content(UpdateNoteContentRequest {
            note_id: NOTE_ID.to_owned(),
            title: TITLE_MARKER.to_owned(),
            body_format: "tiptap-json".to_owned(),
            body_schema_version: 1,
            body_json: changed_body,
            base_revision: 1,
            client_change_id: CHANGE_ID.to_owned(),
            content_hash: calculate_content_hash_hex(
                "2026-09-06",
                TITLE_MARKER,
                &canonical_changed,
            ),
        })
        .unwrap();
    assert_eq!(updated.revision, 2);
    assert_eq!(updated.id, NOTE_ID);
    assert_eq!(updated.created_at_ms, created.created_at_ms);

    let mut invalid_update = desktop_notes_core::NoteUpdate {
        id: NOTE_ID.to_owned(),
        title: "x".repeat(501),
        body_json: updated.body_json.clone(),
        body_text: updated.body_text.clone(),
        content_hash: updated.content_hash,
        updated_at_ms: updated.updated_at_ms + 1,
        base_revision: updated.revision,
    };
    assert!(reopened.update_content(invalid_update.clone()).is_err());
    invalid_update.title.clear();
    assert_eq!(reopened.get(NOTE_ID).unwrap().unwrap(), updated);

    for candidate in [
        path.clone(),
        PathBuf::from(format!("{}-wal", path.display())),
        PathBuf::from(format!("{}-shm", path.display())),
    ] {
        if candidate.exists() {
            let bytes = fs::read(&candidate).unwrap();
            for marker in [TITLE_MARKER.as_bytes(), BODY_MARKER.as_bytes()] {
                assert!(
                    !bytes.windows(marker.len()).any(|window| window == marker),
                    "plaintext marker leaked to {}",
                    candidate.display()
                );
            }
        }
    }
    drop(reopened);

    let before_wrong_key = fs::read(&path).unwrap();
    let wrong = SqlCipherStore::new(path.clone(), "0.1.0");
    let error = wrong.open_or_initialize(&key(0x24)).unwrap_err();
    assert_eq!(error.code(), ErrorCode::DatabaseWrongKey);
    assert_eq!(fs::read(&path).unwrap(), before_wrong_key);
    drop(wrong);

    fs::remove_dir_all(path.parent().unwrap()).unwrap();
}

#[test]
fn encrypted_repository_soft_delete_survives_reopen_and_leaves_active_views_empty() {
    let path = isolated_database();
    let store = SqlCipherStore::new(path.clone(), "0.1.0");
    store.open_or_initialize(&key(0x4d)).unwrap();
    let clock = SequenceClock(Mutex::new(vec![1_780_000_000_000, 1_780_000_000_500]));
    let service = NoteService::new(&store, &FixedId, &clock);
    let created = service
        .create_note(CreateNoteRequest {
            note_date: "2026-09-06".to_owned(),
            title: "Move to recycle".to_owned(),
            body_format: "tiptap-json".to_owned(),
            body_schema_version: 1,
            body_json: paragraph("Encrypted content remains recoverable"),
        })
        .unwrap();

    service
        .delete_note(DeleteNoteRequest {
            note_id: created.id.clone(),
            base_revision: created.revision,
            client_change_id: "80afcd5d-1d76-4580-8ed8-4292a3b77a8b".to_owned(),
        })
        .unwrap();
    assert_eq!(
        service.get_note(&created.id).unwrap_err().code(),
        ErrorCode::NoteNotFound
    );
    assert!(
        service
            .list_notes_for_date("2026-09-06")
            .unwrap()
            .is_empty()
    );
    assert!(
        store
            .count_by_date_range("2026-09-01", "2026-10-01")
            .unwrap()
            .is_empty()
    );
    drop(store);

    let reopened = SqlCipherStore::new(path.clone(), "0.1.0");
    reopened.open_or_initialize(&key(0x4d)).unwrap();
    let reopened_service = NoteService::new(&reopened, &FixedId, &clock);
    assert_eq!(
        reopened_service.get_note(&created.id).unwrap_err().code(),
        ErrorCode::NoteNotFound
    );
    drop(reopened);
    fs::remove_dir_all(path.parent().unwrap()).unwrap();
}

#[test]
fn encrypted_repository_persists_date_counts_moves_and_revision_guarded_undo() {
    let path = isolated_database();
    let store = SqlCipherStore::new(path.clone(), "0.1.0");
    let status = store.open_or_initialize(&key(0x53)).unwrap();
    assert_eq!(status.schema_version, 7);
    let ids = QueueIds(Mutex::new(vec![
        "edcd0114-2781-4208-ae13-c52d92488341".to_owned(),
        "4d5f7f94-bfd4-4cc5-916a-d517aa9d7197".to_owned(),
        "5a5fb977-ab3d-4810-a90a-66f491b62c53".to_owned(),
        "1c27ab31-f9db-41d5-af35-7bf7f9883ca7".to_owned(),
    ]));
    let clock = SequenceClock(Mutex::new(vec![1_000, 1_100, 1_200, 2_000, 2_500]));
    let undo = MemoryUndoStore::default();
    let notes = NoteService::new(&store, &ids, &clock);
    for (date, title) in [
        ("2026-09-06", "First"),
        ("2026-09-06", "Second"),
        ("2026-09-10", "Third"),
    ] {
        notes
            .create_note(CreateNoteRequest {
                note_date: date.to_owned(),
                title: title.to_owned(),
                body_format: "tiptap-json".to_owned(),
                body_schema_version: 1,
                body_json: paragraph("B03 encrypted calendar marker"),
            })
            .unwrap();
    }
    let calendar = CalendarService::new(&store, &ids, &clock, &undo);
    assert_eq!(
        calendar
            .list_note_counts_for_month("2026-09")
            .unwrap()
            .iter()
            .map(|count| (count.note_date.as_str(), count.count))
            .collect::<Vec<_>>(),
        vec![("2026-09-06", 2), ("2026-09-10", 1)]
    );

    let changed = calendar
        .change_note_date(ChangeNoteDateRequest {
            note_id: "edcd0114-2781-4208-ae13-c52d92488341".to_owned(),
            new_note_date: "2026-09-10".to_owned(),
            base_revision: 1,
            client_change_id: "863c1a9d-112c-45ce-988d-b80801a51182".to_owned(),
        })
        .unwrap();
    assert_eq!(changed.note.revision, 2);
    assert_eq!(changed.note.note_date, "2026-09-10");
    drop(store);

    let reopened = SqlCipherStore::new(path.clone(), "0.1.0");
    assert_eq!(
        reopened
            .open_or_initialize(&key(0x53))
            .unwrap()
            .schema_version,
        7
    );
    let reopened_calendar = CalendarService::new(&reopened, &ids, &clock, &undo);
    assert_eq!(
        reopened_calendar
            .list_note_counts_for_month("2026-09")
            .unwrap()
            .iter()
            .map(|count| (count.note_date.as_str(), count.count))
            .collect::<Vec<_>>(),
        vec![("2026-09-06", 1), ("2026-09-10", 2)]
    );
    let stale = reopened_calendar.change_note_date(ChangeNoteDateRequest {
        note_id: changed.note.id.clone(),
        new_note_date: "2026-09-11".to_owned(),
        base_revision: 1,
        client_change_id: "1cb09d6a-c356-4a83-8967-f40fde8764d3".to_owned(),
    });
    assert_eq!(stale.unwrap_err().code(), ErrorCode::RevisionConflict);

    let restored = reopened_calendar
        .undo_note_date_change(UndoNoteDateChangeRequest {
            token_id: changed.undo_token.token_id,
            client_change_id: "19a123bb-8b46-4f69-b931-0421ab13a5f0".to_owned(),
        })
        .unwrap();
    assert_eq!(restored.note_date, "2026-09-06");
    assert_eq!(restored.revision, 3);
    assert_eq!(
        reopened_calendar
            .list_note_counts_for_month("2026-09")
            .unwrap()
            .iter()
            .map(|count| (count.note_date.as_str(), count.count))
            .collect::<Vec<_>>(),
        vec![("2026-09-06", 2), ("2026-09-10", 1)]
    );

    drop(reopened);
    fs::remove_dir_all(path.parent().unwrap()).unwrap();
}
