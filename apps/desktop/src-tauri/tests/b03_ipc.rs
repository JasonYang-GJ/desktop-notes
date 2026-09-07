use std::{collections::HashMap, sync::Mutex};

use desktop_notes_core::{
    Clock, DateCount, DateUndoRecord, DateUndoStore, FoundationError, NewNote, Note,
    NoteDateUpdate, NoteDeletion, NoteIdGenerator, NoteRepository, NoteSummary, NoteUpdate,
};
use desktop_notes_desktop::ipc::{
    handle_change_note_date, handle_list_note_counts_for_month, handle_undo_note_date_change,
};
use serde_json::json;

const NOTE_ID: &str = "d3c0eb9a-80cb-4390-9131-906450613ed4";
const TOKEN_ID: &str = "e52c6c84-e694-46f5-a982-846d9b68fb8b";

struct FixedId;

impl NoteIdGenerator for FixedId {
    fn new_note_id(&self) -> Result<String, FoundationError> {
        Ok(TOKEN_ID.to_owned())
    }
}

struct ClockAt(Mutex<Vec<i64>>);

impl Clock for ClockAt {
    fn now_utc_ms(&self) -> Result<i64, FoundationError> {
        Ok(self.0.lock().unwrap().remove(0))
    }
}

#[derive(Default)]
struct MemoryRepository(Mutex<HashMap<String, Note>>);

impl NoteRepository for MemoryRepository {
    fn create(&self, input: NewNote) -> Result<Note, FoundationError> {
        let note = Note::from_new(input);
        self.0.lock().unwrap().insert(note.id.clone(), note.clone());
        Ok(note)
    }

    fn get(&self, id: &str) -> Result<Option<Note>, FoundationError> {
        Ok(self.0.lock().unwrap().get(id).cloned())
    }

    fn list_for_date(&self, note_date: &str) -> Result<Vec<NoteSummary>, FoundationError> {
        Ok(self
            .0
            .lock()
            .unwrap()
            .values()
            .filter(|note| note.note_date == note_date)
            .map(NoteSummary::from)
            .collect())
    }

    fn count_by_date_range(
        &self,
        start_date: &str,
        end_date_exclusive: &str,
    ) -> Result<Vec<DateCount>, FoundationError> {
        let count = self
            .0
            .lock()
            .unwrap()
            .values()
            .filter(|note| {
                note.note_date.as_str() >= start_date
                    && note.note_date.as_str() < end_date_exclusive
            })
            .count() as u64;
        Ok(if count == 0 {
            vec![]
        } else {
            vec![DateCount {
                note_date: "2026-09-06".to_owned(),
                count,
            }]
        })
    }

    fn update_content(&self, _input: NoteUpdate) -> Result<Note, FoundationError> {
        unreachable!("B03 IPC test does not update content")
    }

    fn update_date(&self, input: NoteDateUpdate) -> Result<Note, FoundationError> {
        let mut notes = self.0.lock().unwrap();
        let note = notes
            .get_mut(&input.id)
            .ok_or_else(FoundationError::note_not_found)?;
        if note.revision != input.base_revision {
            return Err(FoundationError::revision_conflict());
        }
        note.note_date = input.note_date;
        note.content_hash = input.content_hash;
        note.updated_at_ms = input.updated_at_ms;
        note.revision += 1;
        Ok(note.clone())
    }

    fn delete(&self, input: NoteDeletion) -> Result<(), FoundationError> {
        let mut notes = self.0.lock().unwrap();
        let note = notes
            .get(&input.id)
            .ok_or_else(FoundationError::note_not_found)?;
        if note.revision != input.base_revision {
            return Err(FoundationError::revision_conflict());
        }
        notes.remove(&input.id);
        Ok(())
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

fn note() -> Note {
    Note {
        id: NOTE_ID.to_owned(),
        note_date: "2026-09-06".to_owned(),
        title: "B03 calendar note".to_owned(),
        body_json: r#"{"type":"doc","content":[{"type":"paragraph"}]}"#.to_owned(),
        body_format: "tiptap-json".to_owned(),
        body_schema_version: 1,
        body_text: String::new(),
        content_hash: [
            0x15, 0x6f, 0xf1, 0x59, 0x0d, 0x9d, 0xd0, 0x38, 0x03, 0x4a, 0x7e, 0x24, 0xb6, 0x64,
            0x28, 0x05, 0xc0, 0xd3, 0x73, 0xeb, 0x77, 0xdd, 0xd2, 0x6e, 0x5d, 0x5d, 0x1c, 0x0b,
            0xc3, 0x27, 0xb7, 0xe6,
        ],
        is_pinned: false,
        created_at_ms: 1_000,
        updated_at_ms: 1_000,
        revision: 1,
    }
}

#[test]
fn typed_calendar_commands_count_change_and_undo_with_client_identity() {
    let repository = MemoryRepository::default();
    repository
        .0
        .lock()
        .unwrap()
        .insert(NOTE_ID.to_owned(), note());
    let undo = MemoryUndoStore::default();
    let clock = ClockAt(Mutex::new(vec![2_000, 2_500]));
    let service = desktop_notes_core::CalendarService::new(&repository, &FixedId, &clock, &undo);

    let counts = serde_json::to_value(handle_list_note_counts_for_month(
        json!({ "protocolVersion": 1, "month": "2026-09" }),
        &service,
    ))
    .unwrap();
    assert_eq!(counts["ok"], true);
    assert_eq!(counts["counts"][0]["noteDate"], "2026-09-06");
    assert_eq!(counts["counts"][0]["count"], 1);

    let changed = serde_json::to_value(handle_change_note_date(
        json!({
            "protocolVersion": 1,
            "noteId": NOTE_ID,
            "newNoteDate": "2026-09-10",
            "baseRevision": 1,
            "clientChangeId": "6089c54a-6a2a-4916-8a70-986cd12ec46f"
        }),
        &service,
    ))
    .unwrap();
    assert_eq!(changed["ok"], true);
    assert_eq!(changed["note"]["noteDate"], "2026-09-10");
    assert_eq!(changed["previousDate"], "2026-09-06");
    assert_eq!(changed["undoToken"]["tokenId"], TOKEN_ID);
    assert_eq!(changed["undoToken"]["expiresAtMs"], 12_000);
    assert_eq!(
        changed["clientChangeId"],
        "6089c54a-6a2a-4916-8a70-986cd12ec46f"
    );

    let restored = serde_json::to_value(handle_undo_note_date_change(
        json!({
            "protocolVersion": 1,
            "tokenId": TOKEN_ID,
            "clientChangeId": "7860c12d-c89b-477e-9bc7-d779d6cf3c58"
        }),
        &service,
    ))
    .unwrap();
    assert_eq!(restored["ok"], true);
    assert_eq!(restored["note"]["noteDate"], "2026-09-06");
    assert_eq!(restored["note"]["revision"], 3);
    assert_eq!(
        restored["clientChangeId"],
        "7860c12d-c89b-477e-9bc7-d779d6cf3c58"
    );
}

#[test]
fn calendar_commands_reject_unknown_fields_invalid_dates_and_stale_revisions() {
    let repository = MemoryRepository::default();
    repository
        .0
        .lock()
        .unwrap()
        .insert(NOTE_ID.to_owned(), note());
    let undo = MemoryUndoStore::default();
    let clock = ClockAt(Mutex::new(vec![]));
    let service = desktop_notes_core::CalendarService::new(&repository, &FixedId, &clock, &undo);

    for envelope in [
        handle_list_note_counts_for_month(
            json!({ "protocolVersion": 1, "month": "2026-13" }),
            &service,
        ),
        handle_change_note_date(
            json!({
                "protocolVersion": 1,
                "noteId": NOTE_ID,
                "newNoteDate": "2026-02-30",
                "baseRevision": 1,
                "clientChangeId": "6089c54a-6a2a-4916-8a70-986cd12ec46f"
            }),
            &service,
        ),
        handle_change_note_date(
            json!({
                "protocolVersion": 1,
                "noteId": NOTE_ID,
                "newNoteDate": "2026-09-10",
                "baseRevision": 0,
                "clientChangeId": "6089c54a-6a2a-4916-8a70-986cd12ec46f"
            }),
            &service,
        ),
        handle_change_note_date(
            json!({
                "protocolVersion": 1,
                "noteId": NOTE_ID,
                "newNoteDate": "2026-09-10",
                "baseRevision": 1,
                "clientChangeId": "6089c54a-6a2a-4916-8a70-986cd12ec46f",
                "executeSql": "DROP TABLE notes"
            }),
            &service,
        ),
    ] {
        let encoded = serde_json::to_value(envelope).unwrap();
        assert_eq!(encoded["ok"], false);
        assert!(matches!(
            encoded["error"]["code"].as_str(),
            Some("VALIDATION_FAILED" | "REVISION_CONFLICT")
        ));
        assert!(!encoded.to_string().contains("serde"));
    }
}
