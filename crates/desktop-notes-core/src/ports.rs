use serde::{Deserialize, Serialize};

use crate::{
    AssetRecord, DateCount, DateUndoRecord, ErrorCode, FoundationError, ImageInput, NewNote,
    NewTag, Note, NoteDateUpdate, NoteDeletion, NotePinUpdate, NoteSummary, NoteTagUpdate,
    NoteUpdate, PersistedImage, RootKey, SecretKey, StagedAssetImport, Tag, TagRename,
};

pub trait KeyProtector: Send + Sync {
    fn load_key(&self) -> Result<Option<RootKey>, FoundationError>;
    fn create_and_store_key(&self) -> Result<RootKey, FoundationError>;
}

pub trait KeyDeriver: Send + Sync {
    fn derive_database_key(&self, root: &RootKey) -> Result<SecretKey, FoundationError>;
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DatabaseStatus {
    pub newly_created: bool,
    pub schema_version: u32,
    pub cipher_version: String,
}

pub trait EncryptedStore: Send + Sync {
    fn exists(&self) -> Result<bool, FoundationError>;
    fn open_or_initialize(&self, key: &SecretKey) -> Result<DatabaseStatus, FoundationError>;
}

pub trait NoteRepository: Send + Sync {
    fn create(&self, input: NewNote) -> Result<Note, FoundationError>;
    fn get(&self, id: &str) -> Result<Option<Note>, FoundationError>;
    fn list_for_date(&self, note_date: &str) -> Result<Vec<NoteSummary>, FoundationError>;
    fn count_by_date_range(
        &self,
        start_date: &str,
        end_date_exclusive: &str,
    ) -> Result<Vec<DateCount>, FoundationError>;
    fn update_content(&self, input: NoteUpdate) -> Result<Note, FoundationError>;
    fn update_date(&self, input: NoteDateUpdate) -> Result<Note, FoundationError>;
    fn delete(&self, input: NoteDeletion) -> Result<(), FoundationError>;
}

pub trait OrganizationRepository: Send + Sync {
    fn get_note_for_metadata(&self, note_id: &str) -> Result<Option<Note>, FoundationError>;
    fn list_tags(&self) -> Result<Vec<Tag>, FoundationError>;
    fn find_tag_by_normalized_name(
        &self,
        normalized_name: &str,
    ) -> Result<Option<Tag>, FoundationError>;
    fn create_tag(&self, input: NewTag) -> Result<Tag, FoundationError>;
    fn rename_tag(&self, input: TagRename) -> Result<Tag, FoundationError>;
    fn delete_tag(&self, tag_id: &str) -> Result<(), FoundationError>;
    fn list_tags_for_note(&self, note_id: &str) -> Result<Vec<Tag>, FoundationError>;
    fn assign_tag(&self, input: NoteTagUpdate) -> Result<Note, FoundationError>;
    fn remove_tag(&self, input: NoteTagUpdate) -> Result<Note, FoundationError>;
    fn set_pinned(&self, input: NotePinUpdate) -> Result<Note, FoundationError>;
    fn list_recent(&self, limit: u32) -> Result<Vec<NoteSummary>, FoundationError>;
}

pub trait SearchRepository: Send + Sync {
    fn search(&self, query: &str, limit: u32) -> Result<Vec<crate::SearchHit>, FoundationError>;

    fn rebuild_index(&self) -> Result<(), FoundationError>;
}

pub trait AssetStore: Send + Sync {
    fn find_staged_import(
        &self,
        client_import_id: &str,
    ) -> Result<Option<StagedAssetImport>, FoundationError>;
    fn stage_import(&self, staged: StagedAssetImport) -> Result<(), FoundationError>;
    fn discard_staged_import(
        &self,
        client_import_id: &str,
    ) -> Result<Option<StagedAssetImport>, FoundationError>;
    fn get_ready_asset_for_note(
        &self,
        note_id: &str,
        asset_id: &str,
    ) -> Result<Option<AssetRecord>, FoundationError>;
    fn tracked_storage_relpaths(&self) -> Result<Vec<String>, FoundationError>;
    fn list_due_gc_assets(
        &self,
        now_ms: i64,
        limit: u32,
    ) -> Result<Vec<AssetRecord>, FoundationError>;
    fn finalize_gc_asset(&self, asset_id: &str) -> Result<bool, FoundationError>;
    fn record_gc_failure(
        &self,
        asset_id: &str,
        error_code: ErrorCode,
    ) -> Result<(), FoundationError>;
}

pub trait AssetFileStore: Send + Sync {
    fn persist_image(
        &self,
        asset_id: &str,
        input: ImageInput,
    ) -> Result<PersistedImage, FoundationError>;
    fn read_image(
        &self,
        asset_id: &str,
        storage_relpath: &str,
        expected_sha256: &[u8; 32],
        expected_pixel_width: u32,
        expected_pixel_height: u32,
    ) -> Result<Vec<u8>, FoundationError>;
    fn remove_image(&self, asset_id: &str, storage_relpath: &str) -> Result<(), FoundationError>;
    fn cleanup_staging(&self) -> Result<u32, FoundationError>;
    fn quarantine_unknown_assets(
        &self,
        tracked_storage_relpaths: &std::collections::HashSet<String>,
    ) -> Result<u32, FoundationError>;
}

pub trait DateUndoStore: Send + Sync {
    fn remember(&self, record: DateUndoRecord);
    fn get(&self, token_id: &str) -> Option<DateUndoRecord>;
    fn remove(&self, token_id: &str);
}

pub trait NoteIdGenerator: Send + Sync {
    fn new_note_id(&self) -> Result<String, FoundationError>;
}

pub trait AssetIdGenerator: Send + Sync {
    fn new_asset_id(&self) -> Result<String, FoundationError>;
}

pub trait Clock: Send + Sync {
    fn now_utc_ms(&self) -> Result<i64, FoundationError>;
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SafeLogKind {
    FoundationReady,
    FoundationFailed,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SafeLogEvent {
    pub operation_id: String,
    pub module: &'static str,
    pub kind: SafeLogKind,
    pub error_code: Option<crate::ErrorCode>,
    pub duration_ms: u64,
    pub schema_version: Option<u32>,
    pub os_capability: Option<&'static str>,
}

pub trait SafeLogger: Send + Sync {
    fn event(&self, event: SafeLogEvent);
}
