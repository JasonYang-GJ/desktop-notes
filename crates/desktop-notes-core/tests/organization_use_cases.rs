use std::{
    collections::{HashMap, HashSet},
    sync::Mutex,
};

use desktop_notes_core::{
    AssignTagRequest, Clock, ErrorCode, FoundationError, NewNote, NewTag, Note, NoteIdGenerator,
    NotePinUpdate, NoteSummary, NoteTagMutationRequest, NoteTagUpdate, OrganizationRepository,
    OrganizationService, RenameTagRequest, SetPinnedRequest, Tag, TagRename,
};

const TAG_ID: &str = "8d735aa5-4fc9-4abd-bf59-a7d54a9be731";

struct FixedId;

impl NoteIdGenerator for FixedId {
    fn new_note_id(&self) -> Result<String, FoundationError> {
        Ok(TAG_ID.to_owned())
    }
}

struct FixedClock;

impl Clock for FixedClock {
    fn now_utc_ms(&self) -> Result<i64, FoundationError> {
        Ok(1_788_000_000_000)
    }
}

#[derive(Default)]
struct MemoryOrganizationRepository {
    tags: Mutex<HashMap<String, Tag>>,
    notes: Mutex<HashMap<String, Note>>,
    note_tags: Mutex<HashSet<(String, String)>>,
}

impl OrganizationRepository for MemoryOrganizationRepository {
    fn get_note_for_metadata(&self, note_id: &str) -> Result<Option<Note>, FoundationError> {
        Ok(self.notes.lock().unwrap().get(note_id).cloned())
    }

    fn list_tags(&self) -> Result<Vec<Tag>, FoundationError> {
        Ok(self.tags.lock().unwrap().values().cloned().collect())
    }

    fn find_tag_by_normalized_name(
        &self,
        normalized_name: &str,
    ) -> Result<Option<Tag>, FoundationError> {
        Ok(self
            .tags
            .lock()
            .unwrap()
            .values()
            .find(|tag| tag.normalized_name == normalized_name)
            .cloned())
    }

    fn create_tag(&self, input: NewTag) -> Result<Tag, FoundationError> {
        let tag = Tag::from_new(input);
        self.tags
            .lock()
            .unwrap()
            .insert(tag.id.clone(), tag.clone());
        Ok(tag)
    }

    fn rename_tag(&self, input: TagRename) -> Result<Tag, FoundationError> {
        let mut tags = self.tags.lock().unwrap();
        let tag = tags
            .get_mut(&input.id)
            .ok_or_else(FoundationError::tag_not_found)?;
        tag.name = input.name;
        tag.normalized_name = input.normalized_name;
        tag.updated_at_ms = input.updated_at_ms;
        Ok(tag.clone())
    }

    fn delete_tag(&self, tag_id: &str) -> Result<(), FoundationError> {
        self.tags
            .lock()
            .unwrap()
            .remove(tag_id)
            .map(|_| ())
            .ok_or_else(FoundationError::tag_not_found)?;
        self.note_tags
            .lock()
            .unwrap()
            .retain(|(_, assigned_tag_id)| assigned_tag_id != tag_id);
        Ok(())
    }

    fn list_tags_for_note(&self, note_id: &str) -> Result<Vec<Tag>, FoundationError> {
        if !self.notes.lock().unwrap().contains_key(note_id) {
            return Err(FoundationError::note_not_found());
        }
        let assigned = self.note_tags.lock().unwrap();
        let tags = self.tags.lock().unwrap();
        Ok(assigned
            .iter()
            .filter(|(assigned_note_id, _)| assigned_note_id == note_id)
            .filter_map(|(_, tag_id)| tags.get(tag_id).cloned())
            .collect())
    }

    fn assign_tag(&self, input: NoteTagUpdate) -> Result<Note, FoundationError> {
        if !self.tags.lock().unwrap().contains_key(&input.tag_id) {
            return Err(FoundationError::tag_not_found());
        }
        self.mutate_note(input, true)
    }

    fn remove_tag(&self, input: NoteTagUpdate) -> Result<Note, FoundationError> {
        self.mutate_note(input, false)
    }

    fn set_pinned(&self, input: NotePinUpdate) -> Result<Note, FoundationError> {
        let mut notes = self.notes.lock().unwrap();
        let note = notes
            .get_mut(&input.note_id)
            .ok_or_else(FoundationError::note_not_found)?;
        if note.revision != input.base_revision {
            return Err(FoundationError::revision_conflict());
        }
        if note.is_pinned != input.is_pinned {
            note.is_pinned = input.is_pinned;
            note.updated_at_ms = input.updated_at_ms;
            note.revision += 1;
        }
        Ok(note.clone())
    }

    fn list_recent(&self, limit: u32) -> Result<Vec<NoteSummary>, FoundationError> {
        let mut notes: Vec<_> = self
            .notes
            .lock()
            .unwrap()
            .values()
            .map(NoteSummary::from)
            .collect();
        notes.sort_by_key(|left| std::cmp::Reverse(left.updated_at_ms));
        notes.truncate(limit as usize);
        Ok(notes)
    }
}

impl MemoryOrganizationRepository {
    fn mutate_note(&self, input: NoteTagUpdate, assign: bool) -> Result<Note, FoundationError> {
        let mut notes = self.notes.lock().unwrap();
        let note = notes
            .get_mut(&input.note_id)
            .ok_or_else(FoundationError::note_not_found)?;
        if note.revision != input.base_revision {
            return Err(FoundationError::revision_conflict());
        }
        let relation = (input.note_id, input.tag_id);
        let changed = if assign {
            self.note_tags.lock().unwrap().insert(relation)
        } else {
            self.note_tags.lock().unwrap().remove(&relation)
        };
        if changed {
            note.updated_at_ms = input.updated_at_ms;
            note.revision += 1;
        }
        Ok(note.clone())
    }

    fn seed_note(&self) -> Note {
        let note = Note::from_new(NewNote {
            id: "018f6f2a-4b10-7d3a-9b16-1e4eb0af1234".to_owned(),
            note_date: "2026-09-06".to_owned(),
            title: "Body must survive".to_owned(),
            body_json: r#"{"type":"doc","content":[{"type":"paragraph"}]}"#.to_owned(),
            body_text: String::new(),
            content_hash: [7; 32],
            created_at_ms: 1_700_000_000_000,
        });
        self.notes
            .lock()
            .unwrap()
            .insert(note.id.clone(), note.clone());
        note
    }
}

#[test]
fn custom_tag_rename_preserves_stable_id() {
    let repository = MemoryOrganizationRepository::default();
    let service = OrganizationService::new(&repository, &FixedId, &FixedClock);

    let created = service.create_tag("  项目Ａ  ").unwrap();
    assert_eq!(created.id, TAG_ID);
    assert_eq!(created.name, "项目Ａ");
    assert_eq!(created.normalized_name, "项目a");
    assert!(!created.is_seed_default);

    let renamed = service
        .rename_tag(RenameTagRequest {
            tag_id: TAG_ID.to_owned(),
            name: "研究".to_owned(),
        })
        .unwrap();
    assert_eq!(renamed.id, TAG_ID);
    assert_eq!(renamed.name, "研究");
    assert_eq!(renamed.normalized_name, "研究");
}

#[test]
fn duplicate_names_conflict_and_delete_removes_only_the_tag() {
    let repository = MemoryOrganizationRepository::default();
    let service = OrganizationService::new(&repository, &FixedId, &FixedClock);

    let created = service.create_tag("Project").unwrap();
    assert_eq!(
        service.create_tag("  ＰＲＯＪＥＣＴ ").unwrap_err().code(),
        ErrorCode::TagNameConflict,
    );

    service.delete_tag(&created.id).unwrap();
    assert!(service.list_tags().unwrap().is_empty());
    assert_eq!(
        service.delete_tag(&created.id).unwrap_err().code(),
        ErrorCode::TagNotFound,
    );
}

#[test]
fn tag_assignment_removal_and_pin_are_revision_guarded_metadata_mutations() {
    let repository = MemoryOrganizationRepository::default();
    let original = repository.seed_note();
    let service = OrganizationService::new(&repository, &FixedId, &FixedClock);
    let tag = service.create_tag("项目").unwrap();

    let assigned = service
        .assign_tag(AssignTagRequest {
            note_id: original.id.clone(),
            tag_id: tag.id.clone(),
            base_revision: 1,
            client_change_id: "9fc744f5-12a5-4215-900e-dd602f631494".to_owned(),
        })
        .unwrap();
    assert_eq!(assigned.revision, 2);
    assert_eq!(assigned.updated_at_ms, 1_788_000_000_000);
    assert_eq!(assigned.body_json, original.body_json);
    assert_eq!(service.list_tags_for_note(&original.id).unwrap(), vec![tag]);

    assert_eq!(
        service
            .set_pinned(SetPinnedRequest {
                note_id: original.id.clone(),
                is_pinned: true,
                base_revision: 1,
                client_change_id: "8e4b4d31-bc5a-4ec9-8eb4-dab42b60b21a".to_owned(),
            })
            .unwrap_err()
            .code(),
        ErrorCode::RevisionConflict,
    );

    let pinned = service
        .set_pinned(SetPinnedRequest {
            note_id: original.id.clone(),
            is_pinned: true,
            base_revision: 2,
            client_change_id: "ea8b8b6c-d939-4533-89aa-e11333862a4c".to_owned(),
        })
        .unwrap();
    assert!(pinned.is_pinned);
    assert_eq!(pinned.revision, 3);
    assert_eq!(pinned.body_json, original.body_json);

    let removed = service
        .remove_tag(NoteTagMutationRequest {
            note_id: original.id.clone(),
            tag_id: TAG_ID.to_owned(),
            base_revision: 3,
            client_change_id: "d55201bb-ec5a-4b3c-8ca0-8236380dfe6c".to_owned(),
        })
        .unwrap();
    assert_eq!(removed.revision, 4);
    assert!(service.list_tags_for_note(&original.id).unwrap().is_empty());
    assert_eq!(removed.body_json, original.body_json);
}
