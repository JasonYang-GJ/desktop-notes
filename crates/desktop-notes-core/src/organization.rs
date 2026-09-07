use unicode_normalization::UnicodeNormalization;

use crate::FoundationError;

pub const MAX_TAG_NAME_CHARS: usize = 64;
pub const MAX_TAG_NAME_BYTES: usize = 256;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PreparedTagName {
    pub name: String,
    pub normalized_name: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Tag {
    pub id: String,
    pub name: String,
    pub normalized_name: String,
    pub is_seed_default: bool,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

impl Tag {
    pub fn from_new(input: NewTag) -> Self {
        Self {
            id: input.id,
            name: input.name,
            normalized_name: input.normalized_name,
            is_seed_default: false,
            created_at_ms: input.created_at_ms,
            updated_at_ms: input.created_at_ms,
        }
    }

    pub fn validate_stored(&self) -> Result<(), FoundationError> {
        crate::validate_note_id(&self.id)?;
        let prepared = prepare_tag_name(&self.name)?;
        if prepared.name != self.name
            || prepared.normalized_name != self.normalized_name
            || self.created_at_ms < 0
            || self.updated_at_ms < self.created_at_ms
        {
            return Err(FoundationError::validation_failed());
        }
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub struct NewTag {
    pub id: String,
    pub name: String,
    pub normalized_name: String,
    pub created_at_ms: i64,
}

#[derive(Clone, Debug)]
pub struct RenameTagRequest {
    pub tag_id: String,
    pub name: String,
}

#[derive(Clone, Debug)]
pub struct NoteTagMutationRequest {
    pub note_id: String,
    pub tag_id: String,
    pub base_revision: u64,
    pub client_change_id: String,
}

pub type AssignTagRequest = NoteTagMutationRequest;

#[derive(Clone, Debug)]
pub struct SetPinnedRequest {
    pub note_id: String,
    pub is_pinned: bool,
    pub base_revision: u64,
    pub client_change_id: String,
}

#[derive(Clone, Debug)]
pub struct TagRename {
    pub id: String,
    pub name: String,
    pub normalized_name: String,
    pub updated_at_ms: i64,
}

#[derive(Clone, Debug)]
pub struct NoteTagUpdate {
    pub note_id: String,
    pub tag_id: String,
    pub updated_at_ms: i64,
    pub base_revision: u64,
}

#[derive(Clone, Debug)]
pub struct NotePinUpdate {
    pub note_id: String,
    pub is_pinned: bool,
    pub updated_at_ms: i64,
    pub base_revision: u64,
}

pub fn prepare_tag_name(value: &str) -> Result<PreparedTagName, FoundationError> {
    let name = value.trim();
    if name.is_empty()
        || name.chars().count() > MAX_TAG_NAME_CHARS
        || name.len() > MAX_TAG_NAME_BYTES
    {
        return Err(FoundationError::validation_failed());
    }

    let normalized_name: String = name.nfkc().flat_map(char::to_lowercase).collect();
    if normalized_name.is_empty()
        || normalized_name.chars().count() > MAX_TAG_NAME_CHARS
        || normalized_name.len() > MAX_TAG_NAME_BYTES
    {
        return Err(FoundationError::validation_failed());
    }

    Ok(PreparedTagName {
        name: name.to_owned(),
        normalized_name,
    })
}
