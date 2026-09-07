use crate::{
    Clock, CreateNoteRequest, DeleteNoteRequest, FoundationError, NewNote, Note, NoteDeletion,
    NoteIdGenerator, NoteRepository, NoteSummary, NoteUpdate, UpdateNoteContentRequest,
    notes::{decode_content_hash, prepare_content, validate_client_change_id},
    validate_note_date, validate_note_id,
};

pub struct NoteService<'a> {
    repository: &'a dyn NoteRepository,
    ids: &'a dyn NoteIdGenerator,
    clock: &'a dyn Clock,
}

impl<'a> NoteService<'a> {
    pub fn new(
        repository: &'a dyn NoteRepository,
        ids: &'a dyn NoteIdGenerator,
        clock: &'a dyn Clock,
    ) -> Self {
        Self {
            repository,
            ids,
            clock,
        }
    }

    pub fn create_note(&self, request: CreateNoteRequest) -> Result<Note, FoundationError> {
        let prepared = prepare_content(
            &request.note_date,
            &request.title,
            &request.body_format,
            request.body_schema_version,
            &request.body_json,
        )?;
        let id = self.ids.new_note_id()?;
        validate_note_id(&id)?;
        let created_at_ms = self.clock.now_utc_ms()?;
        if created_at_ms < 0 {
            return Err(FoundationError::validation_failed());
        }
        self.repository.create(NewNote {
            id,
            note_date: request.note_date,
            title: request.title,
            body_json: prepared.canonical_json,
            body_text: prepared.body_text,
            content_hash: prepared.content_hash,
            created_at_ms,
        })
    }

    pub fn get_note(&self, id: &str) -> Result<Note, FoundationError> {
        validate_note_id(id)?;
        self.repository
            .get(id)?
            .ok_or_else(FoundationError::note_not_found)
    }

    pub fn list_notes_for_date(
        &self,
        note_date: &str,
    ) -> Result<Vec<NoteSummary>, FoundationError> {
        validate_note_date(note_date)?;
        self.repository.list_for_date(note_date)
    }

    pub fn update_note_content(
        &self,
        request: UpdateNoteContentRequest,
    ) -> Result<Note, FoundationError> {
        validate_note_id(&request.note_id)?;
        validate_client_change_id(&request.client_change_id)?;
        let existing = self
            .repository
            .get(&request.note_id)?
            .ok_or_else(FoundationError::note_not_found)?;
        if existing.revision != request.base_revision {
            return Err(FoundationError::revision_conflict());
        }
        let prepared = prepare_content(
            &existing.note_date,
            &request.title,
            &request.body_format,
            request.body_schema_version,
            &request.body_json,
        )?;
        if decode_content_hash(&request.content_hash)? != prepared.content_hash {
            return Err(FoundationError::validation_failed());
        }
        let updated_at_ms = self.clock.now_utc_ms()?;
        if updated_at_ms < existing.updated_at_ms {
            return Err(FoundationError::validation_failed());
        }
        self.repository.update_content(NoteUpdate {
            id: request.note_id,
            title: request.title,
            body_json: prepared.canonical_json,
            body_text: prepared.body_text,
            content_hash: prepared.content_hash,
            updated_at_ms,
            base_revision: request.base_revision,
        })
    }

    pub fn delete_note(&self, request: DeleteNoteRequest) -> Result<(), FoundationError> {
        validate_note_id(&request.note_id)?;
        validate_client_change_id(&request.client_change_id)?;
        let existing = self
            .repository
            .get(&request.note_id)?
            .ok_or_else(FoundationError::note_not_found)?;
        if existing.revision != request.base_revision {
            return Err(FoundationError::revision_conflict());
        }
        let deleted_at_ms = self.clock.now_utc_ms()?;
        if deleted_at_ms < existing.updated_at_ms {
            return Err(FoundationError::validation_failed());
        }
        self.repository.delete(NoteDeletion {
            id: request.note_id,
            deleted_at_ms,
            base_revision: request.base_revision,
        })
    }
}
