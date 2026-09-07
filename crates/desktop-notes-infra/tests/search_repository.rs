use std::{
    collections::{HashMap, VecDeque},
    fs,
    path::PathBuf,
    sync::Mutex,
    time::SystemTime,
};

use desktop_notes_core::{
    AssignTagRequest, CalendarService, ChangeNoteDateRequest, Clock, CreateNoteRequest,
    DateUndoRecord, DateUndoStore, EncryptedStore, ErrorCode, FoundationError, NoteIdGenerator,
    NoteService, NoteTagMutationRequest, OrganizationService, RenameTagRequest, SearchService,
    SecretKey, UpdateNoteContentRequest, calculate_content_hash_hex,
};
use desktop_notes_infra::{MigrationRegistry, SqlCipherStore};
use rusqlite::{Connection, OpenFlags};
use serde_json::json;

const NOTE_ID: &str = "10000000-0000-4000-8000-000000000001";
const SECOND_NOTE_ID: &str = "10000000-0000-4000-8000-000000000002";
const TAG_ID: &str = "20000000-0000-4000-8000-000000000001";
const UNDO_ID: &str = "30000000-0000-4000-8000-000000000001";

struct QueueIds(Mutex<VecDeque<String>>);

impl QueueIds {
    fn new(ids: &[&str]) -> Self {
        Self(Mutex::new(ids.iter().map(|id| (*id).to_owned()).collect()))
    }
}

impl NoteIdGenerator for QueueIds {
    fn new_note_id(&self) -> Result<String, FoundationError> {
        self.0
            .lock()
            .unwrap()
            .pop_front()
            .ok_or_else(FoundationError::validation_failed)
    }
}

struct SequenceClock(Mutex<VecDeque<i64>>);

impl SequenceClock {
    fn new(values: &[i64]) -> Self {
        Self(Mutex::new(values.iter().copied().collect()))
    }
}

impl Clock for SequenceClock {
    fn now_utc_ms(&self) -> Result<i64, FoundationError> {
        self.0
            .lock()
            .unwrap()
            .pop_front()
            .ok_or_else(FoundationError::validation_failed)
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

fn isolated_database(label: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir()
        .join(format!(
            "desktop-notes-b06-{label}-{}-{nonce}",
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

fn paragraph(text: &str) -> serde_json::Value {
    json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [{ "type": "text", "text": text }]
        }]
    })
}

fn assert_single_hit(search: &SearchService<'_>, query: &str, expected_id: &str) {
    let hits = search.search(query).unwrap();
    assert_eq!(hits.len(), 1, "unexpected results for {query:?}");
    assert_eq!(hits[0].note.id, expected_id);
}

#[test]
fn fts_and_escaped_like_cover_language_length_wildcard_and_syntax_edges_without_mutation() {
    let path = isolated_database("query-boundaries");
    let store = SqlCipherStore::new(path.clone(), "0.1.0");
    assert_eq!(
        store.open_or_initialize(&key(0x71)).unwrap().schema_version,
        7
    );
    let ids = QueueIds::new(&[NOTE_ID]);
    let clock = SequenceClock::new(&[1_000]);
    let notes = NoteService::new(&store, &ids, &clock);
    notes
        .create_note(CreateNoteRequest {
            note_date: "2026-09-06".to_owned(),
            title: "Alpha 天气预报".to_owned(),
            body_format: "tiptap-json".to_owned(),
            body_schema_version: 1,
            body_json: paragraph(
                "天气Search mixed body; literal 100% and under_score; quotes \" and \"\"\"; FTS NEAR(foo) syntax; digits 12345; code fn_main(); URL https://example.test/a; emoji 📝; MixedCaseToken.",
            ),
        })
        .unwrap();
    let before = notes.get_note(NOTE_ID).unwrap();
    let search = SearchService::new(&store);

    for query in [
        "天",
        "天气",
        "天气预报",
        "A",
        "Al",
        "Alp",
        "天气Search",
        "%",
        "_",
        "\"",
        "\"\"\"",
        "NEAR(foo)",
        "123",
        "fn_",
        "https://example",
        "📝",
        "mixedcasetoken",
    ] {
        assert_single_hit(&search, query, NOTE_ID);
    }
    assert!(search.search("' OR 1=1 --").unwrap().is_empty());
    assert!(search.search("   ").unwrap().is_empty());
    assert_eq!(
        search.search(&"界".repeat(257)).unwrap_err().code(),
        ErrorCode::ValidationFailed
    );

    let after = notes.get_note(NOTE_ID).unwrap();
    assert_eq!(after, before, "search changed persisted Note state");
    drop(store);
    fs::remove_dir_all(path.parent().unwrap()).unwrap();
}

#[test]
fn migration_and_triggers_keep_title_body_tags_and_date_consistent_across_restart() {
    let path = isolated_database("index-lifecycle");
    let b05 = SqlCipherStore::with_registry(path.clone(), "0.1.0", MigrationRegistry::b05());
    assert_eq!(
        b05.open_or_initialize(&key(0x72)).unwrap().schema_version,
        3
    );
    let ids = QueueIds::new(&[NOTE_ID]);
    let create_clock = SequenceClock::new(&[1_000]);
    NoteService::new(&b05, &ids, &create_clock)
        .create_note(CreateNoteRequest {
            note_date: "2026-09-06".to_owned(),
            title: "Legacy title marker".to_owned(),
            body_format: "tiptap-json".to_owned(),
            body_schema_version: 1,
            body_json: paragraph("legacy body marker"),
        })
        .unwrap();
    drop(b05);

    let store = SqlCipherStore::new(path.clone(), "0.1.0");
    assert_eq!(
        store.open_or_initialize(&key(0x72)).unwrap().schema_version,
        7
    );
    let search = SearchService::new(&store);
    assert_single_hit(&search, "Legacy title", NOTE_ID);
    assert_single_hit(&search, "legacy body", NOTE_ID);

    let mutation_ids = QueueIds::new(&[TAG_ID, UNDO_ID]);
    let mutation_clock = SequenceClock::new(&[1_100, 1_200, 1_300, 1_400, 1_500, 1_600, 1_700]);
    let notes = NoteService::new(&store, &mutation_ids, &mutation_clock);
    let updated_doc = paragraph("autosave final searchable body");
    let updated_json = serde_json::to_string(&updated_doc).unwrap();
    let updated = notes
        .update_note_content(UpdateNoteContentRequest {
            note_id: NOTE_ID.to_owned(),
            title: "Updated searchable title".to_owned(),
            body_format: "tiptap-json".to_owned(),
            body_schema_version: 1,
            body_json: updated_doc,
            base_revision: 1,
            client_change_id: "40000000-0000-4000-8000-000000000001".to_owned(),
            content_hash: calculate_content_hash_hex(
                "2026-09-06",
                "Updated searchable title",
                &updated_json,
            ),
        })
        .unwrap();
    assert_eq!(updated.revision, 2);
    assert!(search.search("Legacy title").unwrap().is_empty());
    assert!(search.search("legacy body").unwrap().is_empty());
    assert_single_hit(&search, "Updated searchable", NOTE_ID);
    assert_single_hit(&search, "autosave final", NOTE_ID);

    let organization = OrganizationService::new(&store, &mutation_ids, &mutation_clock);
    let tag = organization.create_tag("ProjectAlpha").unwrap();
    assert_eq!(tag.id, TAG_ID);
    let assigned = organization
        .assign_tag(AssignTagRequest {
            note_id: NOTE_ID.to_owned(),
            tag_id: TAG_ID.to_owned(),
            base_revision: updated.revision,
            client_change_id: "40000000-0000-4000-8000-000000000002".to_owned(),
        })
        .unwrap();
    assert_single_hit(&search, "ProjectAlpha", NOTE_ID);
    assert_eq!(
        search.search("ProjectAlpha").unwrap()[0].matching_tags[0].id,
        TAG_ID
    );
    assert_eq!(
        search.search("ProjectAlpha").unwrap()[0].matching_tags[0].name,
        "ProjectAlpha"
    );

    organization
        .rename_tag(RenameTagRequest {
            tag_id: TAG_ID.to_owned(),
            name: "项目标签".to_owned(),
        })
        .unwrap();
    assert!(search.search("ProjectAlpha").unwrap().is_empty());
    assert_single_hit(&search, "项目标签", NOTE_ID);

    let removed = organization
        .remove_tag(NoteTagMutationRequest {
            note_id: NOTE_ID.to_owned(),
            tag_id: TAG_ID.to_owned(),
            base_revision: assigned.revision,
            client_change_id: "40000000-0000-4000-8000-000000000003".to_owned(),
        })
        .unwrap();
    assert!(search.search("项目标签").unwrap().is_empty());
    let reassigned = organization
        .assign_tag(AssignTagRequest {
            note_id: NOTE_ID.to_owned(),
            tag_id: TAG_ID.to_owned(),
            base_revision: removed.revision,
            client_change_id: "40000000-0000-4000-8000-000000000004".to_owned(),
        })
        .unwrap();
    assert_single_hit(&search, "项目标签", NOTE_ID);
    organization.delete_tag(TAG_ID).unwrap();
    assert!(search.search("项目标签").unwrap().is_empty());

    let before_search = notes.get_note(NOTE_ID).unwrap();
    assert_single_hit(&search, "searchable", NOTE_ID);
    assert_eq!(notes.get_note(NOTE_ID).unwrap(), before_search);

    let undo = MemoryUndoStore::default();
    let calendar = CalendarService::new(&store, &mutation_ids, &mutation_clock, &undo);
    let moved = calendar
        .change_note_date(ChangeNoteDateRequest {
            note_id: NOTE_ID.to_owned(),
            new_note_date: "2026-09-07".to_owned(),
            base_revision: reassigned.revision,
            client_change_id: "40000000-0000-4000-8000-000000000005".to_owned(),
        })
        .unwrap();
    let moved_hit = search.search("searchable").unwrap().remove(0);
    assert_eq!(moved_hit.note.id, NOTE_ID);
    assert_eq!(moved_hit.note.note_date, "2026-09-07");
    assert_eq!(moved_hit.note.revision, moved.note.revision);
    drop(store);

    let reopened = SqlCipherStore::new(path.clone(), "0.1.0");
    assert_eq!(
        reopened
            .open_or_initialize(&key(0x72))
            .unwrap()
            .schema_version,
        7
    );
    let reopened_hit = SearchService::new(&reopened)
        .search("autosave final")
        .unwrap()
        .remove(0);
    assert_eq!(reopened_hit.note.id, NOTE_ID);
    assert_eq!(reopened_hit.note.note_date, "2026-09-07");
    assert_eq!(reopened_hit.note.revision, moved.note.revision);
    assert!(
        SearchService::new(&reopened)
            .search("项目标签")
            .unwrap()
            .is_empty()
    );
    drop(reopened);

    let raw = open_raw(&path, 0x72);
    raw.execute("DELETE FROM note_fts", []).unwrap();
    raw.close().unwrap();

    let rebuild_store = SqlCipherStore::new(path.clone(), "0.1.0");
    rebuild_store.open_or_initialize(&key(0x72)).unwrap();
    let no_ids = QueueIds::new(&[]);
    let no_clock = SequenceClock::new(&[]);
    let readable_note = NoteService::new(&rebuild_store, &no_ids, &no_clock)
        .get_note(NOTE_ID)
        .unwrap();
    assert_eq!(readable_note.title, "Updated searchable title");
    let rebuild_search = SearchService::new(&rebuild_store);
    assert!(rebuild_search.search("autosave final").unwrap().is_empty());
    rebuild_search.rebuild_index().unwrap();
    assert_single_hit(&rebuild_search, "autosave final", NOTE_ID);
    drop(rebuild_store);
    fs::remove_dir_all(path.parent().unwrap()).unwrap();
}

#[test]
fn relevance_is_stable_and_result_limit_is_enforced_inside_the_repository_boundary() {
    let path = isolated_database("limit");
    let store = SqlCipherStore::new(path.clone(), "0.1.0");
    store.open_or_initialize(&key(0x73)).unwrap();
    let ids = QueueIds::new(&[NOTE_ID, SECOND_NOTE_ID]);
    let clock = SequenceClock::new(&[1_000, 1_100]);
    let notes = NoteService::new(&store, &ids, &clock);
    for (title, body) in [
        ("common title", "ordinary body"),
        ("ordinary title", "common body"),
    ] {
        notes
            .create_note(CreateNoteRequest {
                note_date: "2026-09-06".to_owned(),
                title: title.to_owned(),
                body_format: "tiptap-json".to_owned(),
                body_schema_version: 1,
                body_json: paragraph(body),
            })
            .unwrap();
    }
    use desktop_notes_core::SearchRepository;
    let first = store.search("common", 1).unwrap();
    assert_eq!(first.len(), 1);
    assert_eq!(first[0].note.id, NOTE_ID, "title match should rank first");
    let all = store.search("common", 1_000).unwrap();
    assert_eq!(all.len(), 2);
    assert_eq!(store.search("common", 1_000).unwrap(), all);
    assert_eq!(store.search("co", 1).unwrap()[0].note.id, NOTE_ID);
    drop(store);
    fs::remove_dir_all(path.parent().unwrap()).unwrap();
}
