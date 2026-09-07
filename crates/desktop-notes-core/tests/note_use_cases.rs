use std::{collections::HashMap, sync::Mutex};

use desktop_notes_core::{
    Clock, CreateNoteRequest, DateCount, DeleteNoteRequest, ErrorCode, FoundationError, NewNote,
    Note, NoteDateUpdate, NoteDeletion, NoteIdGenerator, NoteRepository, NoteService, NoteSummary,
    NoteUpdate, UpdateNoteContentRequest,
};
use serde_json::json;

const NOTE_ID: &str = "018f6f2a-4b10-7d3a-9b16-1e4eb0af1234";

struct FixedId;

impl NoteIdGenerator for FixedId {
    fn new_note_id(&self) -> Result<String, FoundationError> {
        Ok(NOTE_ID.to_owned())
    }
}

struct SequenceClock {
    values: Mutex<Vec<i64>>,
}

impl Clock for SequenceClock {
    fn now_utc_ms(&self) -> Result<i64, FoundationError> {
        Ok(self.values.lock().unwrap().remove(0))
    }
}

#[derive(Default)]
struct MemoryRepository {
    notes: Mutex<HashMap<String, Note>>,
}

impl NoteRepository for MemoryRepository {
    fn create(&self, input: NewNote) -> Result<Note, FoundationError> {
        let note = Note::from_new(input);
        self.notes
            .lock()
            .unwrap()
            .insert(note.id.clone(), note.clone());
        Ok(note)
    }

    fn get(&self, id: &str) -> Result<Option<Note>, FoundationError> {
        Ok(self.notes.lock().unwrap().get(id).cloned())
    }

    fn list_for_date(&self, note_date: &str) -> Result<Vec<NoteSummary>, FoundationError> {
        Ok(self
            .notes
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
        for note in self.notes.lock().unwrap().values() {
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
        let mut notes = self.notes.lock().unwrap();
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
        let mut notes = self.notes.lock().unwrap();
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
        let mut notes = self.notes.lock().unwrap();
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
fn approved_full_rich_text_fixture_is_canonical_and_derives_search_text() {
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "../../../tests/fixtures/rich-text/v1-approved-full.json"
    ))
    .unwrap();
    let repository = MemoryRepository::default();
    let clock = SequenceClock {
        values: Mutex::new(vec![1_780_000_000_000]),
    };
    let service = NoteService::new(&repository, &FixedId, &clock);

    let created = service
        .create_note(CreateNoteRequest {
            note_date: "2026-09-06".to_owned(),
            title: "All approved rich text".to_owned(),
            body_format: "tiptap-json".to_owned(),
            body_schema_version: 1,
            body_json: fixture["body_json"].clone(),
        })
        .unwrap();

    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&created.body_json).unwrap(),
        fixture["body_json"]
    );
    assert_eq!(created.body_text, fixture["expected_body_text"]);
    assert_eq!(service.get_note(NOTE_ID).unwrap(), created);
}

#[test]
fn create_get_list_and_update_preserve_identity_and_revision_contract() {
    let repository = MemoryRepository::default();
    let clock = SequenceClock {
        values: Mutex::new(vec![1_780_000_000_000, 1_780_000_000_750]),
    };
    let service = NoteService::new(&repository, &FixedId, &clock);

    let created = service
        .create_note(CreateNoteRequest {
            note_date: "2026-09-06".to_owned(),
            title: "First".to_owned(),
            body_format: "tiptap-json".to_owned(),
            body_schema_version: 1,
            body_json: paragraph("First line"),
        })
        .unwrap();

    assert_eq!(created.id, NOTE_ID);
    assert_eq!(created.note_date, "2026-09-06");
    assert_eq!(created.body_text, "First line");
    assert_eq!(created.body_format, "tiptap-json");
    assert_eq!(created.body_schema_version, 1);
    assert_eq!(created.revision, 1);
    assert_eq!(created.created_at_ms, 1_780_000_000_000);
    assert_eq!(created.updated_at_ms, 1_780_000_000_000);
    assert_eq!(
        created.content_hash_hex(),
        "bc21b6bd6dc3f3215172572f42ac9f77494fb4979c6849e11b9e7555cfab6ee8"
    );

    assert_eq!(service.get_note(NOTE_ID).unwrap(), created);
    let listed = service.list_notes_for_date("2026-09-06").unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id, NOTE_ID);
    assert_eq!(listed[0].title, "First");

    let updated = service
        .update_note_content(UpdateNoteContentRequest {
            note_id: NOTE_ID.to_owned(),
            title: "Renamed".to_owned(),
            body_format: "tiptap-json".to_owned(),
            body_schema_version: 1,
            body_json: paragraph("Changed body"),
            base_revision: 1,
            client_change_id: "2be547a2-cf54-4c26-912f-aedec754558a".to_owned(),
            content_hash: "c29eb2cb0a633c45cf23c0c3a0af100fdc3ebeb8ec466f072648ffe9bc170ed3"
                .to_owned(),
        })
        .unwrap();

    assert_eq!(updated.id, NOTE_ID);
    assert_eq!(updated.note_date, "2026-09-06");
    assert_eq!(updated.title, "Renamed");
    assert_eq!(updated.body_text, "Changed body");
    assert_eq!(updated.created_at_ms, created.created_at_ms);
    assert_eq!(updated.updated_at_ms, 1_780_000_000_750);
    assert_eq!(updated.revision, 2);

    let conflict = service
        .update_note_content(UpdateNoteContentRequest {
            note_id: NOTE_ID.to_owned(),
            title: "Stale".to_owned(),
            body_format: "tiptap-json".to_owned(),
            body_schema_version: 1,
            body_json: paragraph("Stale body"),
            base_revision: 1,
            client_change_id: "d0d97ea7-aa2c-4398-82b6-1ab9dd31d3bc".to_owned(),
            content_hash: "9bf77b7b9df271820e5aab3a9e0f7c92c62c8a90d0ebb0e4965c0b68a0ef8bf0"
                .to_owned(),
        })
        .unwrap_err();
    assert_eq!(conflict.code(), ErrorCode::RevisionConflict);
}

#[test]
fn delete_requires_the_current_revision_and_removes_the_note_from_active_reads() {
    let repository = MemoryRepository::default();
    let clock = SequenceClock {
        values: Mutex::new(vec![
            1_780_000_000_000,
            1_780_000_000_500,
            1_780_000_000_750,
        ]),
    };
    let service = NoteService::new(&repository, &FixedId, &clock);
    let created = service
        .create_note(CreateNoteRequest {
            note_date: "2026-09-06".to_owned(),
            title: "Delete me".to_owned(),
            body_format: "tiptap-json".to_owned(),
            body_schema_version: 1,
            body_json: paragraph("Still recoverable from recycle storage"),
        })
        .unwrap();

    let stale = service
        .delete_note(DeleteNoteRequest {
            note_id: created.id.clone(),
            base_revision: 2,
            client_change_id: "96fb70ef-9d83-4260-bc0e-3e1d79f514f4".to_owned(),
        })
        .unwrap_err();
    assert_eq!(stale.code(), ErrorCode::RevisionConflict);

    service
        .delete_note(DeleteNoteRequest {
            note_id: created.id.clone(),
            base_revision: 1,
            client_change_id: "cc683170-7285-483c-8536-4fae009bdd9f".to_owned(),
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
}

#[test]
fn invalid_date_unknown_body_and_bad_identifiers_fail_before_persistence() {
    let repository = MemoryRepository::default();
    let clock = SequenceClock {
        values: Mutex::new(vec![1_780_000_000_000]),
    };
    let service = NoteService::new(&repository, &FixedId, &clock);

    for invalid in [
        CreateNoteRequest {
            note_date: "2026-02-30".to_owned(),
            title: String::new(),
            body_format: "tiptap-json".to_owned(),
            body_schema_version: 1,
            body_json: paragraph("body"),
        },
        CreateNoteRequest {
            note_date: "2026-09-06".to_owned(),
            title: String::new(),
            body_format: "tiptap-json".to_owned(),
            body_schema_version: 1,
            body_json: json!({ "type": "doc", "content": [{ "type": "unknown" }] }),
        },
        CreateNoteRequest {
            note_date: "2026-09-06".to_owned(),
            title: String::new(),
            body_format: "tiptap-json".to_owned(),
            body_schema_version: 1,
            body_json: json!({
                "type": "doc",
                "content": [{
                    "type": "paragraph",
                    "content": [{
                        "type": "text",
                        "text": "malformed",
                        "marks": [{ "type": "link", "attrs": { "href": "http://?missing-host" } }]
                    }]
                }]
            }),
        },
    ] {
        assert_eq!(
            service.create_note(invalid).unwrap_err().code(),
            ErrorCode::ValidationFailed
        );
    }

    assert_eq!(
        service.get_note("not-a-uuid").unwrap_err().code(),
        ErrorCode::ValidationFailed
    );
    assert!(repository.notes.lock().unwrap().is_empty());
}
