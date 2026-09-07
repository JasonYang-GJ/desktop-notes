use std::{collections::HashMap, sync::Mutex};

use desktop_notes_core::{
    Clock, DateCount, FoundationError, NewNote, Note, NoteDateUpdate, NoteDeletion,
    NoteIdGenerator, NoteRepository, NoteService, NoteSummary, NoteUpdate,
    calculate_content_hash_hex,
};
use desktop_notes_desktop::ipc::{
    handle_create_note, handle_delete_note, handle_get_note, handle_list_notes_for_date,
    handle_update_note_content,
};
use serde_json::{Value, json};

const NOTE_ID: &str = "b30256a2-a873-4589-aadc-d8e63fc7fb4d";

struct FixedId;

impl NoteIdGenerator for FixedId {
    fn new_note_id(&self) -> Result<String, FoundationError> {
        Ok(NOTE_ID.to_owned())
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
        let mut counts = HashMap::<String, u64>::new();
        for note in self.0.lock().unwrap().values() {
            if note.note_date.as_str() >= start_date && note.note_date.as_str() < end_date_exclusive
            {
                *counts.entry(note.note_date.clone()).or_default() += 1;
            }
        }
        Ok(counts
            .into_iter()
            .map(|(note_date, count)| DateCount { note_date, count })
            .collect())
    }

    fn update_content(&self, input: NoteUpdate) -> Result<Note, FoundationError> {
        let mut notes = self.0.lock().unwrap();
        let note = notes
            .get_mut(&input.id)
            .ok_or_else(FoundationError::note_not_found)?;
        if note.revision != input.base_revision {
            return Err(FoundationError::revision_conflict());
        }
        note.title = input.title;
        note.body_json = input.body_json;
        note.body_text = input.body_text;
        note.content_hash = input.content_hash;
        note.updated_at_ms = input.updated_at_ms;
        note.revision += 1;
        Ok(note.clone())
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

fn body(text: &str) -> Value {
    json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [{ "type": "text", "text": text }]
        }]
    })
}

fn service<'a>(repository: &'a MemoryRepository, clock: &'a ClockAt) -> NoteService<'a> {
    NoteService::new(repository, &FixedId, clock)
}

#[test]
fn typed_note_commands_create_get_list_and_update_with_client_change_identity() {
    let repository = MemoryRepository::default();
    let clock = ClockAt(Mutex::new(vec![
        1_780_000_000_000,
        1_780_000_000_750,
        1_780_000_001_000,
    ]));
    let service = service(&repository, &clock);

    let created = serde_json::to_value(handle_create_note(
        json!({
            "protocolVersion": 1,
            "noteDate": "2026-09-06",
            "title": "B02 Test Note",
            "bodyFormat": "tiptap-json",
            "bodySchemaVersion": 1,
            "bodyJson": body("B02 synthetic body marker")
        }),
        &service,
    ))
    .unwrap();
    assert_eq!(created["ok"], true);
    assert_eq!(created["note"]["id"], NOTE_ID);
    assert_eq!(created["note"]["revision"], 1);

    let listed = serde_json::to_value(handle_list_notes_for_date(
        json!({ "protocolVersion": 1, "noteDate": "2026-09-06" }),
        &service,
    ))
    .unwrap();
    assert_eq!(listed["ok"], true);
    assert_eq!(listed["notes"][0]["title"], "B02 Test Note");

    let loaded = serde_json::to_value(handle_get_note(
        json!({ "protocolVersion": 1, "noteId": NOTE_ID }),
        &service,
    ))
    .unwrap();
    assert_eq!(loaded["note"]["bodyText"], "B02 synthetic body marker");

    let updated_body = body("B02 updated synthetic body marker");
    let canonical = serde_json::to_string(&updated_body).unwrap();
    let updated = serde_json::to_value(handle_update_note_content(
        json!({
            "protocolVersion": 1,
            "noteId": NOTE_ID,
            "title": "B02 Test Note",
            "bodyFormat": "tiptap-json",
            "bodySchemaVersion": 1,
            "bodyJson": updated_body,
            "baseRevision": 1,
            "clientChangeId": "ec0ec3f0-c0ca-4f64-8ab4-350512a41b71",
            "contentHash": calculate_content_hash_hex("2026-09-06", "B02 Test Note", &canonical)
        }),
        &service,
    ))
    .unwrap();
    assert_eq!(updated["ok"], true);
    assert_eq!(
        updated["clientChangeId"],
        "ec0ec3f0-c0ca-4f64-8ab4-350512a41b71"
    );
    assert_eq!(updated["note"]["revision"], 2);

    let deleted = serde_json::to_value(handle_delete_note(
        json!({
            "protocolVersion": 1,
            "noteId": NOTE_ID,
            "baseRevision": 2,
            "clientChangeId": "d71b6de2-d58d-4b42-9384-7291f6099114"
        }),
        &service,
    ))
    .unwrap();
    assert_eq!(deleted["ok"], true);
    assert_eq!(deleted["deletedNoteId"], NOTE_ID);
    assert_eq!(
        deleted["clientChangeId"],
        "d71b6de2-d58d-4b42-9384-7291f6099114"
    );
    assert!(repository.0.lock().unwrap().is_empty());
}

#[test]
fn invalid_ids_dates_schema_and_oversized_payloads_return_controlled_errors() {
    let repository = MemoryRepository::default();
    let clock = ClockAt(Mutex::new(vec![1_780_000_000_000]));
    let service = service(&repository, &clock);

    let invalid = [
        handle_get_note(
            json!({ "protocolVersion": 1, "noteId": "not-a-uuid" }),
            &service,
        ),
        handle_create_note(
            json!({
                "protocolVersion": 1,
                "noteDate": "2026-02-30",
                "title": "",
                "bodyFormat": "tiptap-json",
                "bodySchemaVersion": 1,
                "bodyJson": body("body")
            }),
            &service,
        ),
        handle_create_note(
            json!({
                "protocolVersion": 1,
                "noteDate": "2026-09-06",
                "title": "",
                "bodyFormat": "tiptap-json",
                "bodySchemaVersion": 2,
                "bodyJson": body("body")
            }),
            &service,
        ),
        handle_create_note(
            json!({
                "protocolVersion": 1,
                "noteDate": "2026-09-06",
                "title": "x".repeat(501),
                "bodyFormat": "tiptap-json",
                "bodySchemaVersion": 1,
                "bodyJson": body("body")
            }),
            &service,
        ),
        handle_create_note(
            json!({
                "protocolVersion": 1,
                "noteDate": "2026-09-06",
                "title": "",
                "bodyFormat": "tiptap-json",
                "bodySchemaVersion": 1,
                "bodyJson": body(&"x".repeat(1024 * 1024))
            }),
            &service,
        ),
    ];

    for envelope in invalid {
        let encoded = serde_json::to_value(envelope).unwrap();
        assert_eq!(encoded["ok"], false);
        assert_eq!(encoded["error"]["code"], "VALIDATION_FAILED");
        assert_eq!(
            encoded["error"]["message"],
            "The note request did not match the supported format."
        );
        assert!(!encoded.to_string().contains("serde"));
    }

    let unknown = serde_json::to_value(handle_create_note(
        json!({
            "protocolVersion": 1,
            "noteDate": "2026-09-06",
            "title": "",
            "bodyFormat": "tiptap-json",
            "bodySchemaVersion": 1,
            "bodyJson": body("body"),
            "executeSql": "DROP TABLE notes"
        }),
        &service,
    ))
    .unwrap();
    assert_eq!(unknown["error"]["code"], "VALIDATION_FAILED");
}
