use std::{
    collections::{HashMap, HashSet},
    sync::Mutex,
};

use desktop_notes_core::{
    Clock, FoundationError, NewTag, Note, NoteIdGenerator, NotePinUpdate, NoteSummary,
    NoteTagUpdate, OrganizationRepository, OrganizationService, Tag, TagRename,
};
use desktop_notes_desktop::ipc::{
    handle_assign_tag, handle_create_tag, handle_delete_tag, handle_list_recent_notes,
    handle_list_tags, handle_list_tags_for_note, handle_remove_tag, handle_rename_tag,
    handle_set_note_pinned,
};
use serde_json::json;

const NOTE_ID: &str = "0e67ecfd-17f2-48f0-83c1-c10921016dbb";
const TAG_ID: &str = "aa303a9a-fc89-4586-8f2c-73a556e5ed5e";

struct FixedId;

impl NoteIdGenerator for FixedId {
    fn new_note_id(&self) -> Result<String, FoundationError> {
        Ok(TAG_ID.to_owned())
    }
}

struct ClockAt(Mutex<Vec<i64>>);

impl Clock for ClockAt {
    fn now_utc_ms(&self) -> Result<i64, FoundationError> {
        Ok(self.0.lock().unwrap().remove(0))
    }
}

#[derive(Default)]
struct State {
    notes: HashMap<String, Note>,
    tags: HashMap<String, Tag>,
    relations: HashSet<(String, String)>,
}

struct MemoryRepository(Mutex<State>);

impl MemoryRepository {
    fn with_note() -> Self {
        let note = Note {
            id: NOTE_ID.to_owned(),
            note_date: "2026-09-06".to_owned(),
            title: "B05 IPC".to_owned(),
            body_json: r#"{"type":"doc","content":[{"type":"paragraph"}]}"#.to_owned(),
            body_format: "tiptap-json".to_owned(),
            body_schema_version: 1,
            body_text: String::new(),
            content_hash: [0; 32],
            is_pinned: false,
            created_at_ms: 100,
            updated_at_ms: 100,
            revision: 1,
        };
        let mut state = State::default();
        state.notes.insert(note.id.clone(), note);
        Self(Mutex::new(state))
    }

    fn mutate_note(
        state: &mut State,
        note_id: &str,
        base_revision: u64,
        updated_at_ms: i64,
    ) -> Result<Note, FoundationError> {
        let note = state
            .notes
            .get_mut(note_id)
            .ok_or_else(FoundationError::note_not_found)?;
        if note.revision != base_revision {
            return Err(FoundationError::revision_conflict());
        }
        note.revision += 1;
        note.updated_at_ms = updated_at_ms;
        Ok(note.clone())
    }
}

impl OrganizationRepository for MemoryRepository {
    fn get_note_for_metadata(&self, note_id: &str) -> Result<Option<Note>, FoundationError> {
        Ok(self.0.lock().unwrap().notes.get(note_id).cloned())
    }

    fn list_tags(&self) -> Result<Vec<Tag>, FoundationError> {
        let mut tags: Vec<_> = self.0.lock().unwrap().tags.values().cloned().collect();
        tags.sort_by(|left, right| left.name.cmp(&right.name));
        Ok(tags)
    }

    fn find_tag_by_normalized_name(
        &self,
        normalized_name: &str,
    ) -> Result<Option<Tag>, FoundationError> {
        Ok(self
            .0
            .lock()
            .unwrap()
            .tags
            .values()
            .find(|tag| tag.normalized_name == normalized_name)
            .cloned())
    }

    fn create_tag(&self, input: NewTag) -> Result<Tag, FoundationError> {
        let tag = Tag::from_new(input);
        self.0
            .lock()
            .unwrap()
            .tags
            .insert(tag.id.clone(), tag.clone());
        Ok(tag)
    }

    fn rename_tag(&self, input: TagRename) -> Result<Tag, FoundationError> {
        let mut state = self.0.lock().unwrap();
        let tag = state
            .tags
            .get_mut(&input.id)
            .ok_or_else(FoundationError::tag_not_found)?;
        tag.name = input.name;
        tag.normalized_name = input.normalized_name;
        tag.updated_at_ms = input.updated_at_ms;
        Ok(tag.clone())
    }

    fn delete_tag(&self, tag_id: &str) -> Result<(), FoundationError> {
        let mut state = self.0.lock().unwrap();
        if state.tags.remove(tag_id).is_none() {
            return Err(FoundationError::tag_not_found());
        }
        state
            .relations
            .retain(|(_, related_tag)| related_tag != tag_id);
        Ok(())
    }

    fn list_tags_for_note(&self, note_id: &str) -> Result<Vec<Tag>, FoundationError> {
        let state = self.0.lock().unwrap();
        Ok(state
            .relations
            .iter()
            .filter(|(related_note, _)| related_note == note_id)
            .filter_map(|(_, tag_id)| state.tags.get(tag_id).cloned())
            .collect())
    }

    fn assign_tag(&self, input: NoteTagUpdate) -> Result<Note, FoundationError> {
        let mut state = self.0.lock().unwrap();
        if !state.tags.contains_key(&input.tag_id) {
            return Err(FoundationError::tag_not_found());
        }
        state
            .relations
            .insert((input.note_id.clone(), input.tag_id));
        Self::mutate_note(
            &mut state,
            &input.note_id,
            input.base_revision,
            input.updated_at_ms,
        )
    }

    fn remove_tag(&self, input: NoteTagUpdate) -> Result<Note, FoundationError> {
        let mut state = self.0.lock().unwrap();
        state
            .relations
            .remove(&(input.note_id.clone(), input.tag_id));
        Self::mutate_note(
            &mut state,
            &input.note_id,
            input.base_revision,
            input.updated_at_ms,
        )
    }

    fn set_pinned(&self, input: NotePinUpdate) -> Result<Note, FoundationError> {
        let mut state = self.0.lock().unwrap();
        let note = state
            .notes
            .get_mut(&input.note_id)
            .ok_or_else(FoundationError::note_not_found)?;
        if note.revision != input.base_revision {
            return Err(FoundationError::revision_conflict());
        }
        note.is_pinned = input.is_pinned;
        note.revision += 1;
        note.updated_at_ms = input.updated_at_ms;
        Ok(note.clone())
    }

    fn list_recent(&self, limit: u32) -> Result<Vec<NoteSummary>, FoundationError> {
        let mut notes: Vec<_> = self
            .0
            .lock()
            .unwrap()
            .notes
            .values()
            .map(NoteSummary::from)
            .collect();
        notes.sort_by_key(|left| std::cmp::Reverse(left.updated_at_ms));
        notes.truncate(limit as usize);
        Ok(notes)
    }
}

#[test]
fn typed_organization_commands_preserve_ids_and_echo_metadata_change_identity() {
    let repository = MemoryRepository::with_note();
    let clock = ClockAt(Mutex::new(vec![200, 300, 400, 500, 600]));
    let service = OrganizationService::new(&repository, &FixedId, &clock);

    let created = serde_json::to_value(handle_create_tag(
        json!({ "protocolVersion": 1, "name": "  中文 Tag  " }),
        &service,
    ))
    .unwrap();
    assert_eq!(created["ok"], true);
    assert_eq!(created["tag"]["id"], TAG_ID);
    assert_eq!(created["tag"]["name"], "中文 Tag");

    let renamed = serde_json::to_value(handle_rename_tag(
        json!({ "protocolVersion": 1, "tagId": TAG_ID, "name": "项目" }),
        &service,
    ))
    .unwrap();
    assert_eq!(renamed["tag"]["id"], TAG_ID);
    assert_eq!(renamed["tag"]["name"], "项目");

    let assigned = serde_json::to_value(handle_assign_tag(
        json!({
            "protocolVersion": 1,
            "noteId": NOTE_ID,
            "tagId": TAG_ID,
            "baseRevision": 1,
            "clientChangeId": "0ed50682-84b2-4022-9869-c2a426d90586"
        }),
        &service,
    ))
    .unwrap();
    assert_eq!(assigned["note"]["revision"], 2);
    assert_eq!(assigned["note"]["isPinned"], false);
    assert_eq!(
        assigned["clientChangeId"],
        "0ed50682-84b2-4022-9869-c2a426d90586"
    );

    let note_tags = serde_json::to_value(handle_list_tags_for_note(
        json!({ "protocolVersion": 1, "noteId": NOTE_ID }),
        &service,
    ))
    .unwrap();
    assert_eq!(note_tags["tags"][0]["id"], TAG_ID);

    let pinned = serde_json::to_value(handle_set_note_pinned(
        json!({
            "protocolVersion": 1,
            "noteId": NOTE_ID,
            "isPinned": true,
            "baseRevision": 2,
            "clientChangeId": "a2669878-1a13-4f26-844b-96a23422b3f3"
        }),
        &service,
    ))
    .unwrap();
    assert_eq!(pinned["note"]["isPinned"], true);
    assert_eq!(pinned["note"]["revision"], 3);

    let recent = serde_json::to_value(handle_list_recent_notes(
        json!({ "protocolVersion": 1 }),
        &service,
    ))
    .unwrap();
    assert_eq!(recent["notes"][0]["id"], NOTE_ID);
    assert_eq!(recent["notes"][0]["isPinned"], true);

    let removed = serde_json::to_value(handle_remove_tag(
        json!({
            "protocolVersion": 1,
            "noteId": NOTE_ID,
            "tagId": TAG_ID,
            "baseRevision": 3,
            "clientChangeId": "cd6f06c5-b52b-41cf-894e-890122b05f12"
        }),
        &service,
    ))
    .unwrap();
    assert_eq!(removed["note"]["revision"], 4);

    let deleted = serde_json::to_value(handle_delete_tag(
        json!({ "protocolVersion": 1, "tagId": TAG_ID }),
        &service,
    ))
    .unwrap();
    assert_eq!(deleted["deletedTagId"], TAG_ID);

    let tags =
        serde_json::to_value(handle_list_tags(json!({ "protocolVersion": 1 }), &service)).unwrap();
    assert_eq!(tags["tags"].as_array().unwrap().len(), 0);
}

#[test]
fn organization_commands_reject_unknown_fields_and_stale_metadata_revisions() {
    let repository = MemoryRepository::with_note();
    let clock = ClockAt(Mutex::new(vec![]));
    let service = OrganizationService::new(&repository, &FixedId, &clock);

    let unknown = serde_json::to_value(handle_create_tag(
        json!({ "protocolVersion": 1, "name": "Project", "executeSql": "DROP TABLE tags" }),
        &service,
    ))
    .unwrap();
    assert_eq!(unknown["error"]["code"], "VALIDATION_FAILED");

    let stale = serde_json::to_value(handle_set_note_pinned(
        json!({
            "protocolVersion": 1,
            "noteId": NOTE_ID,
            "isPinned": true,
            "baseRevision": 7,
            "clientChangeId": "b6c01d6b-5c9a-4425-9017-d30a048c7439"
        }),
        &service,
    ))
    .unwrap();
    assert_eq!(stale["error"]["code"], "REVISION_CONFLICT");
}
