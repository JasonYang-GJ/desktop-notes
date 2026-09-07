use std::{collections::HashMap, fs, path::PathBuf, sync::Mutex, time::SystemTime};

use desktop_notes_core::{
    AssignTagRequest, CalendarService, ChangeNoteDateRequest, Clock, CreateNoteRequest,
    DateUndoRecord, DateUndoStore, EncryptedStore, FoundationError, NoteIdGenerator, NoteService,
    NoteTagMutationRequest, OrganizationService, RenameTagRequest, SecretKey, SetPinnedRequest,
    UpdateNoteContentRequest, calculate_content_hash_hex,
};
use desktop_notes_infra::{MigrationRegistry, SqlCipherStore};
use serde_json::json;

const NOTE_ID: &str = "6f5d8b90-72de-4361-bd36-e7628c5dac75";

struct FixedId;

impl NoteIdGenerator for FixedId {
    fn new_note_id(&self) -> Result<String, FoundationError> {
        Ok(NOTE_ID.to_owned())
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

struct FixedClock(Mutex<Vec<i64>>);

impl Clock for FixedClock {
    fn now_utc_ms(&self) -> Result<i64, FoundationError> {
        Ok(self.0.lock().unwrap().remove(0))
    }
}

fn isolated_database() -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir()
        .join(format!(
            "desktop-notes-b05-organization-{}-{nonce}",
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
fn b04_database_migrates_without_note_loss_and_seeds_stable_default_tags_once() {
    let path = isolated_database();
    let b04 = SqlCipherStore::with_registry(path.clone(), "0.1.0", MigrationRegistry::b02());
    assert_eq!(
        b04.open_or_initialize(&key(0x65)).unwrap().schema_version,
        2
    );
    let clock = FixedClock(Mutex::new(vec![1_788_000_000_000]));
    let created = NoteService::new(&b04, &FixedId, &clock)
        .create_note(CreateNoteRequest {
            note_date: "2026-09-06".to_owned(),
            title: "B04 note survives B05 migration".to_owned(),
            body_format: "tiptap-json".to_owned(),
            body_schema_version: 1,
            body_json: json!({
                "type": "doc",
                "content": [{
                    "type": "paragraph",
                    "content": [{ "type": "text", "text": "rich body survives" }]
                }]
            }),
        })
        .unwrap();
    drop(b04);

    let b05 = SqlCipherStore::with_registry(path.clone(), "0.1.0", MigrationRegistry::b05());
    assert_eq!(
        b05.open_or_initialize(&key(0x65)).unwrap().schema_version,
        3
    );
    let restored = NoteService::new(&b05, &FixedId, &FixedClock(Mutex::new(vec![])))
        .get_note(NOTE_ID)
        .unwrap();
    assert_eq!(restored.id, created.id);
    assert_eq!(restored.note_date, created.note_date);
    assert_eq!(restored.title, created.title);
    assert_eq!(restored.body_json, created.body_json);
    assert_eq!(restored.revision, created.revision);

    let tags = OrganizationService::new(&b05, &FixedId, &FixedClock(Mutex::new(vec![])))
        .list_tags()
        .unwrap();
    assert_eq!(tags.len(), 2);
    assert!(tags.iter().all(|tag| tag.is_seed_default));
    assert_eq!(
        tags.iter().map(|tag| tag.name.as_str()).collect::<Vec<_>>(),
        vec!["Personal", "Work"],
    );
    let stable_ids = tags.iter().map(|tag| tag.id.clone()).collect::<Vec<_>>();
    drop(b05);

    let reopened = SqlCipherStore::with_registry(path.clone(), "0.1.0", MigrationRegistry::b05());
    assert_eq!(
        reopened
            .open_or_initialize(&key(0x65))
            .unwrap()
            .schema_version,
        3
    );
    let reopened_tags =
        OrganizationService::new(&reopened, &FixedId, &FixedClock(Mutex::new(vec![])))
            .list_tags()
            .unwrap();
    assert_eq!(reopened_tags.len(), 2);
    assert_eq!(
        reopened_tags
            .into_iter()
            .map(|tag| tag.id)
            .collect::<Vec<_>>(),
        stable_ids,
    );

    drop(reopened);
    fs::remove_dir_all(path.parent().unwrap()).unwrap();
}

#[test]
fn tags_and_pin_use_minimal_revision_guarded_transactions_and_survive_date_move() {
    let path = isolated_database();
    let store = SqlCipherStore::new(path.clone(), "0.1.0");
    assert_eq!(
        store.open_or_initialize(&key(0x66)).unwrap().schema_version,
        7
    );
    let note_ids = QueueIds(Mutex::new(vec![
        "50000000-0000-4000-8000-000000000002".to_owned(),
        "50000000-0000-4000-8000-000000000001".to_owned(),
    ]));
    let create_clock = FixedClock(Mutex::new(vec![1_788_000_000_200, 1_788_000_000_100]));
    let notes = NoteService::new(&store, &note_ids, &create_clock);
    let newer = notes
        .create_note(CreateNoteRequest {
            note_date: "2026-09-06".to_owned(),
            title: "Newer unpinned".to_owned(),
            body_format: "tiptap-json".to_owned(),
            body_schema_version: 1,
            body_json: paragraph("newer body"),
        })
        .unwrap();
    let original = notes
        .create_note(CreateNoteRequest {
            note_date: "2026-09-06".to_owned(),
            title: "Pinned body must survive".to_owned(),
            body_format: "tiptap-json".to_owned(),
            body_schema_version: 1,
            body_json: paragraph("original rich body"),
        })
        .unwrap();

    let tag_ids = QueueIds(Mutex::new(vec![
        "60000000-0000-4000-8000-000000000002".to_owned(),
        "60000000-0000-4000-8000-000000000001".to_owned(),
    ]));
    let metadata_clock = FixedClock(Mutex::new(vec![
        1_788_000_001_000,
        1_788_000_001_100,
        1_788_000_001_200,
        1_788_000_001_300,
        1_788_000_001_400,
        1_788_000_001_500,
        1_788_000_001_600,
        1_788_000_001_800,
    ]));
    let organization = OrganizationService::new(&store, &tag_ids, &metadata_clock);
    let chinese = organization.create_tag(" 中文标签 ").unwrap();
    let second = organization.create_tag("Second").unwrap();
    let assigned = organization
        .assign_tag(AssignTagRequest {
            note_id: original.id.clone(),
            tag_id: chinese.id.clone(),
            base_revision: 1,
            client_change_id: "70000000-0000-4000-8000-000000000001".to_owned(),
        })
        .unwrap();
    let assigned_twice = organization
        .assign_tag(AssignTagRequest {
            note_id: original.id.clone(),
            tag_id: second.id.clone(),
            base_revision: assigned.revision,
            client_change_id: "70000000-0000-4000-8000-000000000002".to_owned(),
        })
        .unwrap();
    let duplicate = organization
        .assign_tag(AssignTagRequest {
            note_id: original.id.clone(),
            tag_id: second.id.clone(),
            base_revision: assigned_twice.revision,
            client_change_id: "70000000-0000-4000-8000-000000000003".to_owned(),
        })
        .unwrap();
    assert_eq!(duplicate.revision, assigned_twice.revision);
    assert_eq!(
        organization.list_tags_for_note(&original.id).unwrap().len(),
        2
    );

    let removed = organization
        .remove_tag(NoteTagMutationRequest {
            note_id: original.id.clone(),
            tag_id: second.id.clone(),
            base_revision: duplicate.revision,
            client_change_id: "70000000-0000-4000-8000-000000000004".to_owned(),
        })
        .unwrap();
    let pinned = organization
        .set_pinned(SetPinnedRequest {
            note_id: original.id.clone(),
            is_pinned: true,
            base_revision: removed.revision,
            client_change_id: "70000000-0000-4000-8000-000000000005".to_owned(),
        })
        .unwrap();
    assert!(pinned.is_pinned);
    assert_eq!(pinned.body_json, original.body_json);
    let date_list = notes.list_notes_for_date("2026-09-06").unwrap();
    assert_eq!(date_list[0].id, original.id);
    assert!(date_list[0].is_pinned);
    assert_eq!(date_list[1].id, newer.id);

    let date_ids = QueueIds(Mutex::new(vec![
        "70000000-0000-4000-8000-000000000006".to_owned(),
    ]));
    let date_clock = FixedClock(Mutex::new(vec![1_788_000_001_700]));
    let undo = MemoryUndoStore::default();
    let moved = CalendarService::new(&store, &date_ids, &date_clock, &undo)
        .change_note_date(ChangeNoteDateRequest {
            note_id: original.id.clone(),
            new_note_date: "2026-09-07".to_owned(),
            base_revision: pinned.revision,
            client_change_id: "70000000-0000-4000-8000-000000000007".to_owned(),
        })
        .unwrap()
        .note;
    assert!(moved.is_pinned);
    assert!(
        !notes
            .list_notes_for_date("2026-09-06")
            .unwrap()
            .iter()
            .any(|note| note.id == original.id)
    );
    assert_eq!(
        notes.list_notes_for_date("2026-09-07").unwrap()[0].id,
        original.id
    );

    let renamed = organization
        .rename_tag(RenameTagRequest {
            tag_id: chinese.id.clone(),
            name: "重命名标签".to_owned(),
        })
        .unwrap();
    assert_eq!(renamed.id, chinese.id);
    assert_eq!(
        organization.list_tags_for_note(&original.id).unwrap(),
        vec![renamed.clone()]
    );
    assert_eq!(
        notes.get_note(&original.id).unwrap().revision,
        moved.revision
    );

    organization.delete_tag(&renamed.id).unwrap();
    assert!(
        organization
            .list_tags_for_note(&original.id)
            .unwrap()
            .is_empty()
    );
    let after_delete = notes.get_note(&original.id).unwrap();
    assert_eq!(after_delete.body_json, original.body_json);
    assert_eq!(after_delete.revision, moved.revision);

    drop(store);
    fs::remove_dir_all(path.parent().unwrap()).unwrap();
}

#[test]
fn recent_tracks_content_updates_and_plain_loads_do_not_change_order() {
    let path = isolated_database();
    let store = SqlCipherStore::new(path.clone(), "0.1.0");
    store.open_or_initialize(&key(0x67)).unwrap();
    let ids = QueueIds(Mutex::new(vec![
        "80000000-0000-4000-8000-000000000003".to_owned(),
        "80000000-0000-4000-8000-000000000002".to_owned(),
        "80000000-0000-4000-8000-000000000001".to_owned(),
    ]));
    let create_clock = FixedClock(Mutex::new(vec![100, 200, 300]));
    let notes = NoteService::new(&store, &ids, &create_clock);
    let a = notes
        .create_note(CreateNoteRequest {
            note_date: "2026-09-01".to_owned(),
            title: "A".to_owned(),
            body_format: "tiptap-json".to_owned(),
            body_schema_version: 1,
            body_json: paragraph("A0"),
        })
        .unwrap();
    let b = notes
        .create_note(CreateNoteRequest {
            note_date: "2026-09-06".to_owned(),
            title: "B".to_owned(),
            body_format: "tiptap-json".to_owned(),
            body_schema_version: 1,
            body_json: paragraph("B0"),
        })
        .unwrap();
    let c = notes
        .create_note(CreateNoteRequest {
            note_date: "2026-09-10".to_owned(),
            title: "C".to_owned(),
            body_format: "tiptap-json".to_owned(),
            body_schema_version: 1,
            body_json: paragraph("C0"),
        })
        .unwrap();
    let update_clock = FixedClock(Mutex::new(vec![400, 500, 600]));
    let updates = NoteService::new(&store, &ids, &update_clock);
    for (note, text, change_id) in [
        (&a, "A1", "90000000-0000-4000-8000-000000000001"),
        (&c, "C1", "90000000-0000-4000-8000-000000000002"),
        (&b, "B1", "90000000-0000-4000-8000-000000000003"),
    ] {
        let body = paragraph(text);
        let canonical = serde_json::to_string(&body).unwrap();
        updates
            .update_note_content(UpdateNoteContentRequest {
                note_id: note.id.clone(),
                title: note.title.clone(),
                body_format: "tiptap-json".to_owned(),
                body_schema_version: 1,
                body_json: body,
                base_revision: note.revision,
                client_change_id: change_id.to_owned(),
                content_hash: calculate_content_hash_hex(&note.note_date, &note.title, &canonical),
            })
            .unwrap();
    }

    let read_clock = FixedClock(Mutex::new(vec![]));
    let organization = OrganizationService::new(&store, &ids, &read_clock);
    let before_load = organization.list_recent().unwrap();
    assert_eq!(
        before_load
            .iter()
            .map(|note| note.title.as_str())
            .collect::<Vec<_>>(),
        vec!["B", "C", "A"]
    );
    updates.get_note(&a.id).unwrap();
    updates.get_note(&c.id).unwrap();
    let after_load = organization.list_recent().unwrap();
    assert_eq!(after_load, before_load);

    drop(store);
    fs::remove_dir_all(path.parent().unwrap()).unwrap();
}
