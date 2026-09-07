use std::{collections::HashMap, sync::Mutex, time::SystemTime};

use desktop_notes_core::{
    AssetIdGenerator, Clock, DateUndoRecord, DateUndoStore, ErrorCode, FoundationError,
    NoteIdGenerator,
};

pub struct SystemClock;

impl Clock for SystemClock {
    fn now_utc_ms(&self) -> Result<i64, FoundationError> {
        let millis = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map_err(|_| clock_error())?
            .as_millis();
        i64::try_from(millis).map_err(|_| clock_error())
    }
}

pub struct UuidV4Generator;

impl NoteIdGenerator for UuidV4Generator {
    fn new_note_id(&self) -> Result<String, FoundationError> {
        new_uuid_v4("A secure Note identifier could not be generated.")
    }
}

impl AssetIdGenerator for UuidV4Generator {
    fn new_asset_id(&self) -> Result<String, FoundationError> {
        new_uuid_v4("A secure image identifier could not be generated.")
    }
}

fn new_uuid_v4(safe_message: &'static str) -> Result<String, FoundationError> {
    let mut bytes = [0_u8; 16];
    getrandom::fill(&mut bytes)
        .map_err(|_| FoundationError::new(ErrorCode::InternalError, safe_message))?;
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    Ok(format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        bytes[0],
        bytes[1],
        bytes[2],
        bytes[3],
        bytes[4],
        bytes[5],
        bytes[6],
        bytes[7],
        bytes[8],
        bytes[9],
        bytes[10],
        bytes[11],
        bytes[12],
        bytes[13],
        bytes[14],
        bytes[15]
    ))
}

#[derive(Default)]
pub struct SessionDateUndoStore {
    records: Mutex<HashMap<String, DateUndoRecord>>,
}

impl DateUndoStore for SessionDateUndoStore {
    fn remember(&self, record: DateUndoRecord) {
        let mut records = self
            .records
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if records.len() >= 64
            && let Some(oldest) = records
                .values()
                .min_by_key(|record| record.expires_at_ms)
                .map(|record| record.token_id.clone())
        {
            records.remove(&oldest);
        }
        records.insert(record.token_id.clone(), record);
    }

    fn get(&self, token_id: &str) -> Option<DateUndoRecord> {
        self.records
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(token_id)
            .cloned()
    }

    fn remove(&self, token_id: &str) {
        self.records
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(token_id);
    }
}

fn clock_error() -> FoundationError {
    FoundationError::new(
        ErrorCode::InternalError,
        "The system clock is unavailable for Note metadata.",
    )
}
