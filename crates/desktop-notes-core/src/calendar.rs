use crate::{
    Clock, DateUndoStore, FoundationError, Note, NoteIdGenerator, NoteRepository,
    notes::{calculate_content_hash, validate_client_change_id},
    validate_note_date, validate_note_id,
};

pub const DATE_CHANGE_UNDO_WINDOW_MS: i64 = 10_000;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DateCount {
    pub note_date: String,
    pub count: u64,
}

#[derive(Clone, Debug)]
pub struct ChangeNoteDateRequest {
    pub note_id: String,
    pub new_note_date: String,
    pub base_revision: u64,
    pub client_change_id: String,
}

#[derive(Clone, Debug)]
pub struct UndoNoteDateChangeRequest {
    pub token_id: String,
    pub client_change_id: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DateChangeUndoToken {
    pub token_id: String,
    pub expires_at_ms: i64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DateChangeReceipt {
    pub note: Note,
    pub previous_date: String,
    pub undo_token: DateChangeUndoToken,
}

#[derive(Clone, Debug)]
pub struct NoteDateUpdate {
    pub id: String,
    pub note_date: String,
    pub content_hash: [u8; 32],
    pub updated_at_ms: i64,
    pub base_revision: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DateUndoRecord {
    pub token_id: String,
    pub note_id: String,
    pub previous_date: String,
    pub changed_revision: u64,
    pub expires_at_ms: i64,
}

pub struct CalendarService<'a> {
    repository: &'a dyn NoteRepository,
    ids: &'a dyn NoteIdGenerator,
    clock: &'a dyn Clock,
    undo: &'a dyn DateUndoStore,
}

impl<'a> CalendarService<'a> {
    pub fn new(
        repository: &'a dyn NoteRepository,
        ids: &'a dyn NoteIdGenerator,
        clock: &'a dyn Clock,
        undo: &'a dyn DateUndoStore,
    ) -> Self {
        Self {
            repository,
            ids,
            clock,
            undo,
        }
    }

    pub fn list_note_counts_for_month(
        &self,
        month: &str,
    ) -> Result<Vec<DateCount>, FoundationError> {
        let (start, end_exclusive) = month_bounds(month)?;
        let counts = self
            .repository
            .count_by_date_range(&start, &end_exclusive)?;
        for count in &counts {
            validate_note_date(&count.note_date)?;
            if count.count == 0
                || count.note_date.as_str() < start.as_str()
                || count.note_date.as_str() >= end_exclusive.as_str()
            {
                return Err(FoundationError::validation_failed());
            }
        }
        Ok(counts)
    }

    pub fn change_note_date(
        &self,
        request: ChangeNoteDateRequest,
    ) -> Result<DateChangeReceipt, FoundationError> {
        validate_note_id(&request.note_id)?;
        validate_note_date(&request.new_note_date)?;
        validate_client_change_id(&request.client_change_id)?;
        let existing = self
            .repository
            .get(&request.note_id)?
            .ok_or_else(FoundationError::note_not_found)?;
        if existing.revision != request.base_revision {
            return Err(FoundationError::revision_conflict());
        }
        if existing.note_date == request.new_note_date {
            return Err(FoundationError::validation_failed());
        }

        let updated_at_ms = self.clock.now_utc_ms()?;
        if updated_at_ms < existing.updated_at_ms {
            return Err(FoundationError::validation_failed());
        }
        let expires_at_ms = updated_at_ms
            .checked_add(DATE_CHANGE_UNDO_WINDOW_MS)
            .ok_or_else(FoundationError::validation_failed)?;
        let token_id = self.ids.new_note_id()?;
        validate_note_id(&token_id)?;
        let changed_revision = existing
            .revision
            .checked_add(1)
            .ok_or_else(FoundationError::validation_failed)?;
        let previous_date = existing.note_date.clone();
        let content_hash =
            calculate_content_hash(&request.new_note_date, &existing.title, &existing.body_json);
        let note = self.repository.update_date(NoteDateUpdate {
            id: request.note_id.clone(),
            note_date: request.new_note_date,
            content_hash,
            updated_at_ms,
            base_revision: request.base_revision,
        })?;
        if note.revision != changed_revision {
            return Err(FoundationError::revision_conflict());
        }

        self.undo.remember(DateUndoRecord {
            token_id: token_id.clone(),
            note_id: request.note_id,
            previous_date: previous_date.clone(),
            changed_revision,
            expires_at_ms,
        });

        Ok(DateChangeReceipt {
            note,
            previous_date,
            undo_token: DateChangeUndoToken {
                token_id,
                expires_at_ms,
            },
        })
    }

    pub fn undo_note_date_change(
        &self,
        request: UndoNoteDateChangeRequest,
    ) -> Result<Note, FoundationError> {
        validate_note_id(&request.token_id)?;
        validate_client_change_id(&request.client_change_id)?;
        let record = self
            .undo
            .get(&request.token_id)
            .ok_or_else(FoundationError::undo_expired)?;
        let now = self.clock.now_utc_ms()?;
        if now >= record.expires_at_ms {
            self.undo.remove(&request.token_id);
            return Err(FoundationError::undo_expired());
        }
        let existing = self
            .repository
            .get(&record.note_id)?
            .ok_or_else(FoundationError::note_not_found)?;
        if existing.revision != record.changed_revision {
            self.undo.remove(&request.token_id);
            return Err(FoundationError::revision_conflict());
        }
        if now < existing.updated_at_ms {
            return Err(FoundationError::validation_failed());
        }
        let content_hash =
            calculate_content_hash(&record.previous_date, &existing.title, &existing.body_json);
        let restored = self.repository.update_date(NoteDateUpdate {
            id: record.note_id,
            note_date: record.previous_date,
            content_hash,
            updated_at_ms: now,
            base_revision: record.changed_revision,
        })?;
        self.undo.remove(&request.token_id);
        Ok(restored)
    }
}

fn month_bounds(month: &str) -> Result<(String, String), FoundationError> {
    let bytes = month.as_bytes();
    if bytes.len() != 7 || bytes[4] != b'-' {
        return Err(FoundationError::validation_failed());
    }
    let year = parse_component(&bytes[..4])?;
    let month_number = parse_component(&bytes[5..])?;
    if year == 0 || month_number == 0 || month_number > 12 {
        return Err(FoundationError::validation_failed());
    }
    let start = format!("{year:04}-{month_number:02}-01");
    let (next_year, next_month) = if month_number == 12 {
        (
            year.checked_add(1)
                .ok_or_else(FoundationError::validation_failed)?,
            1,
        )
    } else {
        (year, month_number + 1)
    };
    if next_year > 9999 {
        return Err(FoundationError::validation_failed());
    }
    Ok((start, format!("{next_year:04}-{next_month:02}-01")))
}

fn parse_component(bytes: &[u8]) -> Result<u32, FoundationError> {
    if !bytes.iter().all(u8::is_ascii_digit) {
        return Err(FoundationError::validation_failed());
    }
    bytes.iter().try_fold(0_u32, |value, byte| {
        value
            .checked_mul(10)
            .and_then(|value| value.checked_add(u32::from(byte - b'0')))
            .ok_or_else(FoundationError::validation_failed)
    })
}
