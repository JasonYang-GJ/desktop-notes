use crate::{
    AssignTagRequest, Clock, FoundationError, NewTag, Note, NoteIdGenerator, NotePinUpdate,
    NoteSummary, NoteTagMutationRequest, NoteTagUpdate, OrganizationRepository, RenameTagRequest,
    SetPinnedRequest, Tag, TagRename, notes::validate_client_change_id, prepare_tag_name,
    validate_note_id,
};

pub struct OrganizationService<'a> {
    repository: &'a dyn OrganizationRepository,
    ids: &'a dyn NoteIdGenerator,
    clock: &'a dyn Clock,
}

impl<'a> OrganizationService<'a> {
    pub fn new(
        repository: &'a dyn OrganizationRepository,
        ids: &'a dyn NoteIdGenerator,
        clock: &'a dyn Clock,
    ) -> Self {
        Self {
            repository,
            ids,
            clock,
        }
    }

    pub fn create_tag(&self, name: &str) -> Result<Tag, FoundationError> {
        let prepared = prepare_tag_name(name)?;
        if self
            .repository
            .find_tag_by_normalized_name(&prepared.normalized_name)?
            .is_some()
        {
            return Err(FoundationError::tag_name_conflict());
        }
        let id = self.ids.new_note_id()?;
        validate_note_id(&id)?;
        let created_at_ms = self.clock.now_utc_ms()?;
        if created_at_ms < 0 {
            return Err(FoundationError::validation_failed());
        }
        self.repository.create_tag(NewTag {
            id,
            name: prepared.name,
            normalized_name: prepared.normalized_name,
            created_at_ms,
        })
    }

    pub fn list_tags(&self) -> Result<Vec<Tag>, FoundationError> {
        self.repository.list_tags()
    }

    pub fn delete_tag(&self, tag_id: &str) -> Result<(), FoundationError> {
        validate_note_id(tag_id)?;
        self.repository.delete_tag(tag_id)
    }

    pub fn list_tags_for_note(&self, note_id: &str) -> Result<Vec<Tag>, FoundationError> {
        validate_note_id(note_id)?;
        self.repository.list_tags_for_note(note_id)
    }

    pub fn assign_tag(&self, request: AssignTagRequest) -> Result<Note, FoundationError> {
        self.mutate_tag(request, true)
    }

    pub fn remove_tag(&self, request: NoteTagMutationRequest) -> Result<Note, FoundationError> {
        self.mutate_tag(request, false)
    }

    pub fn set_pinned(&self, request: SetPinnedRequest) -> Result<Note, FoundationError> {
        validate_note_id(&request.note_id)?;
        validate_client_change_id(&request.client_change_id)?;
        let updated_at_ms = self.metadata_timestamp(&request.note_id, request.base_revision)?;
        self.repository.set_pinned(NotePinUpdate {
            note_id: request.note_id,
            is_pinned: request.is_pinned,
            updated_at_ms,
            base_revision: request.base_revision,
        })
    }

    pub fn list_recent(&self) -> Result<Vec<NoteSummary>, FoundationError> {
        self.repository.list_recent(100)
    }

    pub fn rename_tag(&self, request: RenameTagRequest) -> Result<Tag, FoundationError> {
        validate_note_id(&request.tag_id)?;
        let prepared = prepare_tag_name(&request.name)?;
        if self
            .repository
            .find_tag_by_normalized_name(&prepared.normalized_name)?
            .is_some_and(|tag| tag.id != request.tag_id)
        {
            return Err(FoundationError::tag_name_conflict());
        }
        let updated_at_ms = self.clock.now_utc_ms()?;
        self.repository.rename_tag(TagRename {
            id: request.tag_id,
            name: prepared.name,
            normalized_name: prepared.normalized_name,
            updated_at_ms,
        })
    }

    fn mutate_tag(
        &self,
        request: NoteTagMutationRequest,
        assign: bool,
    ) -> Result<Note, FoundationError> {
        validate_note_id(&request.note_id)?;
        validate_note_id(&request.tag_id)?;
        validate_client_change_id(&request.client_change_id)?;
        let updated_at_ms = self.metadata_timestamp(&request.note_id, request.base_revision)?;
        let update = NoteTagUpdate {
            note_id: request.note_id,
            tag_id: request.tag_id,
            updated_at_ms,
            base_revision: request.base_revision,
        };
        if assign {
            self.repository.assign_tag(update)
        } else {
            self.repository.remove_tag(update)
        }
    }

    fn metadata_timestamp(
        &self,
        note_id: &str,
        base_revision: u64,
    ) -> Result<i64, FoundationError> {
        if base_revision == 0 {
            return Err(FoundationError::validation_failed());
        }
        let current = self
            .repository
            .get_note_for_metadata(note_id)?
            .ok_or_else(FoundationError::note_not_found)?;
        if current.revision != base_revision {
            return Err(FoundationError::revision_conflict());
        }
        let timestamp = self.clock.now_utc_ms()?;
        if timestamp < current.updated_at_ms {
            return Err(FoundationError::validation_failed());
        }
        Ok(timestamp)
    }
}
