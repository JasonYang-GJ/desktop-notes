use std::{collections::HashMap, sync::Mutex};

use desktop_notes_core::{
    CalendarService, ChangeNoteDateRequest, Clock, CreateNoteRequest, DateCount, DateUndoRecord,
    DateUndoStore, ErrorCode, FoundationError, NewNote, Note, NoteDateUpdate, NoteDeletion,
    NoteIdGenerator, NoteRepository, NoteService, NoteSummary, NoteUpdate,
    UndoNoteDateChangeRequest, UpdateNoteContentRequest, calculate_content_hash_hex,
};
use serde_json::json;

const NOTE_ID: &str = "89f91b7f-977b-494d-a4b8-cb50e5fc211c";
const TOKEN_ID: &str = "d2430847-2195-4ce9-bd87-d3d4797a3e06";

struct QueueIds(Mutex<Vec<String>>);

impl NoteIdGenerator for QueueIds {
    fn new_note_id(&self) -> Result<String, FoundationError> {
        Ok(self.0.lock().unwrap().remove(0))
    }
}

struct SequenceClock(Mutex<Vec<i64>>);

impl Clock for SequenceClock {
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
        let mut counts = counts
            .into_iter()
            .map(|(note_date, count)| DateCount { note_date, count })
            .collect::<Vec<_>>();
        counts.sort_by(|left, right| left.note_date.cmp(&right.note_date));
        Ok(counts)
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
fn date_change_updates_counts_and_revision_and_short_undo_restores_the_original_date() {
    let repository = MemoryRepository::default();
    let undo = MemoryUndoStore::default();
    let ids = QueueIds(Mutex::new(vec![NOTE_ID.to_owned(), TOKEN_ID.to_owned()]));
    let clock = SequenceClock(Mutex::new(vec![1_000, 2_000, 2_500]));
    let notes = NoteService::new(&repository, &ids, &clock);
    let calendar = CalendarService::new(&repository, &ids, &clock, &undo);

    let original = notes
        .create_note(CreateNoteRequest {
            note_date: "2026-09-06".to_owned(),
            title: "Calendar note".to_owned(),
            body_format: "tiptap-json".to_owned(),
            body_schema_version: 1,
            body_json: paragraph("B03 date change marker"),
        })
        .unwrap();
    assert_eq!(
        calendar.list_note_counts_for_month("2026-09").unwrap()[0].count,
        1
    );

    let changed = calendar
        .change_note_date(ChangeNoteDateRequest {
            note_id: NOTE_ID.to_owned(),
            new_note_date: "2026-09-10".to_owned(),
            base_revision: 1,
            client_change_id: "61b80c77-8a6a-40ff-ad4a-f4ebad8c9810".to_owned(),
        })
        .unwrap();

    assert_eq!(changed.previous_date, "2026-09-06");
    assert_eq!(changed.note.note_date, "2026-09-10");
    assert_eq!(changed.note.id, NOTE_ID);
    assert_eq!(changed.note.revision, 2);
    assert_eq!(changed.undo_token.token_id, TOKEN_ID);
    assert_eq!(changed.undo_token.expires_at_ms, 12_000);
    assert_ne!(changed.note.content_hash, original.content_hash);
    assert_eq!(
        calendar.list_note_counts_for_month("2026-09").unwrap(),
        vec![DateCount {
            note_date: "2026-09-10".to_owned(),
            count: 1
        }]
    );

    let restored = calendar
        .undo_note_date_change(UndoNoteDateChangeRequest {
            token_id: TOKEN_ID.to_owned(),
            client_change_id: "7bd4ed72-01c8-42af-9d98-17b9aef98880".to_owned(),
        })
        .unwrap();
    assert_eq!(restored.note_date, "2026-09-06");
    assert_eq!(restored.id, NOTE_ID);
    assert_eq!(restored.revision, 3);
    assert_eq!(restored.content_hash, original.content_hash);
    assert!(undo.get(TOKEN_ID).is_none());
}

#[test]
fn undo_refuses_to_overwrite_content_saved_after_the_date_change() {
    let repository = MemoryRepository::default();
    let undo = MemoryUndoStore::default();
    let ids = QueueIds(Mutex::new(vec![NOTE_ID.to_owned(), TOKEN_ID.to_owned()]));
    let clock = SequenceClock(Mutex::new(vec![1_000, 2_000, 2_500, 3_000]));
    let notes = NoteService::new(&repository, &ids, &clock);
    let calendar = CalendarService::new(&repository, &ids, &clock, &undo);

    notes
        .create_note(CreateNoteRequest {
            note_date: "2026-09-06".to_owned(),
            title: "Calendar note".to_owned(),
            body_format: "tiptap-json".to_owned(),
            body_schema_version: 1,
            body_json: paragraph("Before"),
        })
        .unwrap();
    calendar
        .change_note_date(ChangeNoteDateRequest {
            note_id: NOTE_ID.to_owned(),
            new_note_date: "2026-09-10".to_owned(),
            base_revision: 1,
            client_change_id: "61b80c77-8a6a-40ff-ad4a-f4ebad8c9810".to_owned(),
        })
        .unwrap();
    let changed_body = paragraph("After date move");
    let canonical = serde_json::to_string(&changed_body).unwrap();
    let edited = notes
        .update_note_content(UpdateNoteContentRequest {
            note_id: NOTE_ID.to_owned(),
            title: "Calendar note".to_owned(),
            body_format: "tiptap-json".to_owned(),
            body_schema_version: 1,
            body_json: changed_body,
            base_revision: 2,
            client_change_id: "f5d12226-12b0-49b9-8167-43425788125f".to_owned(),
            content_hash: calculate_content_hash_hex("2026-09-10", "Calendar note", &canonical),
        })
        .unwrap();

    let error = calendar
        .undo_note_date_change(UndoNoteDateChangeRequest {
            token_id: TOKEN_ID.to_owned(),
            client_change_id: "7bd4ed72-01c8-42af-9d98-17b9aef98880".to_owned(),
        })
        .unwrap_err();
    assert_eq!(error.code(), ErrorCode::RevisionConflict);
    assert_eq!(edited.note_date, "2026-09-10");
    assert_eq!(repository.get(NOTE_ID).unwrap().unwrap(), edited);
    assert!(undo.get(TOKEN_ID).is_none());
}

#[test]
fn expired_undo_and_invalid_months_fail_with_controlled_errors() {
    let repository = MemoryRepository::default();
    let undo = MemoryUndoStore::default();
    let ids = QueueIds(Mutex::new(vec![NOTE_ID.to_owned(), TOKEN_ID.to_owned()]));
    let clock = SequenceClock(Mutex::new(vec![1_000, 2_000, 12_000]));
    let notes = NoteService::new(&repository, &ids, &clock);
    let calendar = CalendarService::new(&repository, &ids, &clock, &undo);

    notes
        .create_note(CreateNoteRequest {
            note_date: "2026-12-31".to_owned(),
            title: "Year boundary".to_owned(),
            body_format: "tiptap-json".to_owned(),
            body_schema_version: 1,
            body_json: paragraph("Boundary"),
        })
        .unwrap();
    assert_eq!(
        calendar.list_note_counts_for_month("2026-12").unwrap()[0].count,
        1
    );
    assert!(
        calendar
            .list_note_counts_for_month("2027-01")
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        calendar
            .list_note_counts_for_month("2026-13")
            .unwrap_err()
            .code(),
        ErrorCode::ValidationFailed
    );
    calendar
        .change_note_date(ChangeNoteDateRequest {
            note_id: NOTE_ID.to_owned(),
            new_note_date: "2027-01-01".to_owned(),
            base_revision: 1,
            client_change_id: "61b80c77-8a6a-40ff-ad4a-f4ebad8c9810".to_owned(),
        })
        .unwrap();

    let error = calendar
        .undo_note_date_change(UndoNoteDateChangeRequest {
            token_id: TOKEN_ID.to_owned(),
            client_change_id: "7bd4ed72-01c8-42af-9d98-17b9aef98880".to_owned(),
        })
        .unwrap_err();
    assert_eq!(error.code(), ErrorCode::UndoExpired);
    assert_eq!(
        repository.get(NOTE_ID).unwrap().unwrap().note_date,
        "2027-01-01"
    );
}
