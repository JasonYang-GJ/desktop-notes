use std::fmt;

use base64::{Engine as _, engine::general_purpose::STANDARD};
use desktop_notes_core::{
    CalendarService, CaptureSession, CapturedClipboard, ChangeNoteDateRequest, CreateNoteRequest,
    DateChangeUndoToken, DateCount, DeleteNoteRequest, ErrorCode, FoundationError,
    FoundationStatus, ImageAssetService, ImageInput, ImageInputSource, ImageSourceFormat,
    ImportImageRequest, Note, NoteService, NoteSummary, NoteTagMutationRequest,
    OrganizationService, RenameTagRequest, SearchHit, SearchService, SetPinnedRequest, Tag,
    UndoNoteDateChangeRequest, UpdateNoteContentRequest,
};
use desktop_notes_infra::{BackupHealth, BackupHealthState, MAX_IMAGE_INPUT_BYTES};
use serde::{Deserialize, Serialize, Serializer, ser::SerializeStruct};
use serde_json::Value;

pub const ALLOWED_COMMANDS: [&str; 31] = [
    "get_foundation_status",
    "create_note",
    "get_note",
    "delete_note",
    "list_notes_for_date",
    "update_note_content",
    "list_note_counts_for_month",
    "change_note_date",
    "undo_note_date_change",
    "list_tags",
    "create_tag",
    "rename_tag",
    "delete_tag",
    "list_tags_for_note",
    "assign_tag",
    "remove_tag",
    "set_note_pinned",
    "list_recent_notes",
    "search_notes",
    "import_image_asset",
    "read_image_asset",
    "discard_image_asset",
    "take_quick_capture",
    "finish_quick_capture",
    "get_shortcut_config",
    "set_shortcut_config",
    "get_backup_status",
    "get_window_session",
    "set_visual_state",
    "set_theme_mode",
    "save_window_placement",
];
const IPC_PROTOCOL_VERSION: u16 = 1;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ShortcutRuntimeStatus {
    pub configured_shortcut: String,
    pub active_shortcut: Option<String>,
    pub registration_error: Option<ErrorCode>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct FoundationRequest {
    protocol_version: u16,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct CreateNoteDto {
    protocol_version: u16,
    note_date: String,
    title: String,
    body_format: String,
    body_schema_version: u32,
    body_json: Value,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct GetNoteDto {
    protocol_version: u16,
    note_id: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct DeleteNoteDto {
    protocol_version: u16,
    note_id: String,
    base_revision: u64,
    client_change_id: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct ListNotesForDateDto {
    protocol_version: u16,
    note_date: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct UpdateNoteContentDto {
    protocol_version: u16,
    note_id: String,
    title: String,
    body_format: String,
    body_schema_version: u32,
    body_json: Value,
    base_revision: u64,
    client_change_id: String,
    content_hash: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct ListNoteCountsForMonthDto {
    protocol_version: u16,
    month: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct ChangeNoteDateDto {
    protocol_version: u16,
    note_id: String,
    new_note_date: String,
    base_revision: u64,
    client_change_id: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct UndoNoteDateChangeDto {
    protocol_version: u16,
    token_id: String,
    client_change_id: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct ListTagsDto {
    protocol_version: u16,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct CreateTagDto {
    protocol_version: u16,
    name: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct RenameTagDto {
    protocol_version: u16,
    tag_id: String,
    name: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct DeleteTagDto {
    protocol_version: u16,
    tag_id: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct ListTagsForNoteDto {
    protocol_version: u16,
    note_id: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct NoteTagMutationDto {
    protocol_version: u16,
    note_id: String,
    tag_id: String,
    base_revision: u64,
    client_change_id: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct SetNotePinnedDto {
    protocol_version: u16,
    note_id: String,
    is_pinned: bool,
    base_revision: u64,
    client_change_id: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct ListRecentNotesDto {
    protocol_version: u16,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct SearchNotesDto {
    protocol_version: u16,
    query: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct ImportImageAssetDto {
    protocol_version: u16,
    client_import_id: String,
    media_type: String,
    source: String,
    data_base64: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct ReadImageAssetDto {
    protocol_version: u16,
    note_id: String,
    asset_id: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct DiscardImageAssetDto {
    protocol_version: u16,
    client_import_id: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct TakeQuickCaptureDto {
    protocol_version: u16,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct FinishQuickCaptureDto {
    protocol_version: u16,
    session_id: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct GetShortcutConfigDto {
    protocol_version: u16,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct SetShortcutConfigDto {
    protocol_version: u16,
    shortcut: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct GetBackupStatusDto {
    protocol_version: u16,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FoundationStatusDto {
    protocol_version: u16,
    ready: bool,
    view: &'static str,
    database_state: desktop_notes_core::DatabaseState,
    schema_version: u32,
    encryption: &'static str,
    backup_status: Option<BackupStatusDto>,
}

impl From<FoundationStatus> for FoundationStatusDto {
    fn from(status: FoundationStatus) -> Self {
        Self {
            protocol_version: IPC_PROTOCOL_VERSION,
            ready: status.ready,
            view: status.view,
            database_state: status.database_state,
            schema_version: status.schema_version,
            encryption: status.encryption,
            backup_status: None,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IpcErrorDto {
    pub code: ErrorCode,
    pub message: String,
    pub recoverable: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NoteDto {
    protocol_version: u16,
    id: String,
    note_date: String,
    title: String,
    body_json: Value,
    body_format: String,
    body_schema_version: u32,
    body_text: String,
    content_hash: String,
    is_pinned: bool,
    created_at_ms: i64,
    updated_at_ms: i64,
    revision: u64,
}

impl TryFrom<Note> for NoteDto {
    type Error = FoundationError;

    fn try_from(note: Note) -> Result<Self, Self::Error> {
        let content_hash = note.content_hash_hex();
        let body_json = serde_json::from_str(&note.body_json).map_err(|_| {
            FoundationError::new(
                ErrorCode::InternalError,
                "The stored Note could not be returned safely.",
            )
        })?;
        Ok(Self {
            protocol_version: IPC_PROTOCOL_VERSION,
            id: note.id,
            note_date: note.note_date,
            title: note.title,
            body_json,
            body_format: note.body_format,
            body_schema_version: note.body_schema_version,
            body_text: note.body_text,
            content_hash,
            is_pinned: note.is_pinned,
            created_at_ms: note.created_at_ms,
            updated_at_ms: note.updated_at_ms,
            revision: note.revision,
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NoteSummaryDto {
    protocol_version: u16,
    id: String,
    note_date: String,
    title: String,
    is_pinned: bool,
    updated_at_ms: i64,
    revision: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchHitDto {
    protocol_version: u16,
    id: String,
    note_date: String,
    title: String,
    snippet: String,
    matching_tags: Vec<SearchMatchTagDto>,
    updated_at_ms: i64,
    revision: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchMatchTagDto {
    id: String,
    name: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageAssetDto {
    protocol_version: u16,
    asset_id: String,
    media_type: String,
    pixel_width: u32,
    pixel_height: u32,
    data_base64: String,
}

impl From<desktop_notes_core::DisplayImage> for ImageAssetDto {
    fn from(image: desktop_notes_core::DisplayImage) -> Self {
        Self {
            protocol_version: IPC_PROTOCOL_VERSION,
            asset_id: image.asset_id,
            media_type: image.media_type,
            pixel_width: image.pixel_width,
            pixel_height: image.pixel_height,
            data_base64: STANDARD.encode(image.bytes),
        }
    }
}

impl From<SearchHit> for SearchHitDto {
    fn from(hit: SearchHit) -> Self {
        Self {
            protocol_version: IPC_PROTOCOL_VERSION,
            id: hit.note.id,
            note_date: hit.note.note_date,
            title: hit.note.title,
            snippet: hit.snippet,
            matching_tags: hit
                .matching_tags
                .into_iter()
                .map(|tag| SearchMatchTagDto {
                    id: tag.id,
                    name: tag.name,
                })
                .collect(),
            updated_at_ms: hit.note.updated_at_ms,
            revision: hit.note.revision,
        }
    }
}

impl From<NoteSummary> for NoteSummaryDto {
    fn from(note: NoteSummary) -> Self {
        Self {
            protocol_version: IPC_PROTOCOL_VERSION,
            id: note.id,
            note_date: note.note_date,
            title: note.title,
            is_pinned: note.is_pinned,
            updated_at_ms: note.updated_at_ms,
            revision: note.revision,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TagDto {
    protocol_version: u16,
    id: String,
    name: String,
    is_seed_default: bool,
    created_at_ms: i64,
    updated_at_ms: i64,
}

impl From<Tag> for TagDto {
    fn from(tag: Tag) -> Self {
        Self {
            protocol_version: IPC_PROTOCOL_VERSION,
            id: tag.id,
            name: tag.name,
            is_seed_default: tag.is_seed_default,
            created_at_ms: tag.created_at_ms,
            updated_at_ms: tag.updated_at_ms,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DateCountDto {
    note_date: String,
    count: u64,
}

impl From<DateCount> for DateCountDto {
    fn from(count: DateCount) -> Self {
        Self {
            note_date: count.note_date,
            count: count.count,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DateChangeUndoTokenDto {
    token_id: String,
    expires_at_ms: i64,
}

#[derive(Clone, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QuickCaptureSessionDto {
    protocol_version: u16,
    id: String,
    content_type: &'static str,
    text: Option<String>,
    data_base64: Option<String>,
    media_type: Option<&'static str>,
    acquisition_attempts: u8,
}

impl From<CaptureSession> for QuickCaptureSessionDto {
    fn from(session: CaptureSession) -> Self {
        let (content_type, text, data_base64, media_type) = match session.clipboard {
            CapturedClipboard::Text(text) => ("text", Some(text), None, None),
            CapturedClipboard::ImagePng(bytes) => (
                "image",
                None,
                Some(STANDARD.encode(bytes)),
                Some("image/png"),
            ),
            CapturedClipboard::TextAndImage { text, png } => (
                "text_and_image",
                Some(text),
                Some(STANDARD.encode(png)),
                Some("image/png"),
            ),
            CapturedClipboard::Empty => ("empty", None, None, None),
            CapturedClipboard::Unsupported => ("unsupported", None, None, None),
            CapturedClipboard::Busy => ("busy", None, None, None),
            CapturedClipboard::Failed => ("failed", None, None, None),
        };
        Self {
            protocol_version: IPC_PROTOCOL_VERSION,
            id: session.id,
            content_type,
            text,
            data_base64,
            media_type,
            acquisition_attempts: session.acquisition_attempts,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ShortcutStatusDto {
    protocol_version: u16,
    configured_shortcut: String,
    active_shortcut: Option<String>,
    registration_error: Option<ErrorCode>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupStatusDto {
    protocol_version: u16,
    state: BackupHealthState,
    last_success_at_ms: Option<i64>,
    last_success_local_day: Option<String>,
    valid_generation_count: usize,
    last_error_code: Option<ErrorCode>,
}

impl From<BackupHealth> for BackupStatusDto {
    fn from(status: BackupHealth) -> Self {
        Self {
            protocol_version: IPC_PROTOCOL_VERSION,
            state: status.state,
            last_success_at_ms: status.last_success_at_ms,
            last_success_local_day: status.last_success_local_day,
            valid_generation_count: status.valid_generation_count,
            last_error_code: status.last_error_code,
        }
    }
}

impl From<ShortcutRuntimeStatus> for ShortcutStatusDto {
    fn from(status: ShortcutRuntimeStatus) -> Self {
        Self {
            protocol_version: IPC_PROTOCOL_VERSION,
            configured_shortcut: status.configured_shortcut,
            active_shortcut: status.active_shortcut,
            registration_error: status.registration_error,
        }
    }
}

impl From<DateChangeUndoToken> for DateChangeUndoTokenDto {
    fn from(token: DateChangeUndoToken) -> Self {
        Self {
            token_id: token.token_id,
            expires_at_ms: token.expires_at_ms,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FoundationEnvelope {
    Ready { status: FoundationStatusDto },
    Error { error: IpcErrorDto },
}

impl Serialize for FoundationEnvelope {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            Self::Ready { status } => {
                let mut state = serializer.serialize_struct("FoundationEnvelope", 2)?;
                state.serialize_field("ok", &true)?;
                state.serialize_field("status", status)?;
                state.end()
            }
            Self::Error { error } => {
                let mut state = serializer.serialize_struct("FoundationEnvelope", 2)?;
                state.serialize_field("ok", &false)?;
                state.serialize_field("error", error)?;
                state.end()
            }
        }
    }
}

#[derive(Clone, Eq, PartialEq)]
pub enum CaptureEnvelope {
    Session {
        session: Option<QuickCaptureSessionDto>,
    },
    Error {
        error: IpcErrorDto,
    },
}

impl Serialize for CaptureEnvelope {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            Self::Session { session } => {
                let mut state = serializer.serialize_struct("CaptureEnvelope", 2)?;
                state.serialize_field("ok", &true)?;
                state.serialize_field("session", session)?;
                state.end()
            }
            Self::Error { error } => {
                let mut state = serializer.serialize_struct("CaptureEnvelope", 2)?;
                state.serialize_field("ok", &false)?;
                state.serialize_field("error", error)?;
                state.end()
            }
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ShortcutEnvelope {
    Status { status: ShortcutStatusDto },
    Error { error: IpcErrorDto },
}

impl Serialize for ShortcutEnvelope {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            Self::Status { status } => {
                let mut state = serializer.serialize_struct("ShortcutEnvelope", 2)?;
                state.serialize_field("ok", &true)?;
                state.serialize_field("status", status)?;
                state.end()
            }
            Self::Error { error } => {
                let mut state = serializer.serialize_struct("ShortcutEnvelope", 2)?;
                state.serialize_field("ok", &false)?;
                state.serialize_field("error", error)?;
                state.end()
            }
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BackupEnvelope {
    Status { status: BackupStatusDto },
    Error { error: IpcErrorDto },
}

impl Serialize for BackupEnvelope {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            Self::Status { status } => {
                let mut state = serializer.serialize_struct("BackupEnvelope", 2)?;
                state.serialize_field("ok", &true)?;
                state.serialize_field("status", status)?;
                state.end()
            }
            Self::Error { error } => {
                let mut state = serializer.serialize_struct("BackupEnvelope", 2)?;
                state.serialize_field("ok", &false)?;
                state.serialize_field("error", error)?;
                state.end()
            }
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ActionEnvelope {
    Done,
    Error { error: IpcErrorDto },
}

impl Serialize for ActionEnvelope {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            Self::Done => {
                let mut state = serializer.serialize_struct("ActionEnvelope", 1)?;
                state.serialize_field("ok", &true)?;
                state.end()
            }
            Self::Error { error } => {
                let mut state = serializer.serialize_struct("ActionEnvelope", 2)?;
                state.serialize_field("ok", &false)?;
                state.serialize_field("error", error)?;
                state.end()
            }
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum NoteEnvelope {
    Note {
        note: NoteDto,
    },
    Notes {
        notes: Vec<NoteSummaryDto>,
    },
    SearchHits {
        hits: Vec<SearchHitDto>,
    },
    ImageAsset {
        image: ImageAssetDto,
    },
    Updated {
        note: NoteDto,
        client_change_id: String,
    },
    DateCounts {
        counts: Vec<DateCountDto>,
    },
    DateChanged {
        note: NoteDto,
        previous_date: String,
        undo_token: DateChangeUndoTokenDto,
        client_change_id: String,
    },
    DateUndoApplied {
        note: NoteDto,
        client_change_id: String,
    },
    Tags {
        tags: Vec<TagDto>,
    },
    Tag {
        tag: TagDto,
    },
    TagDeleted {
        deleted_tag_id: String,
    },
    NoteDeleted {
        deleted_note_id: String,
        client_change_id: String,
    },
    MetadataUpdated {
        note: NoteDto,
        client_change_id: String,
    },
    Error {
        error: IpcErrorDto,
    },
}

impl Serialize for NoteEnvelope {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            Self::Note { note } => {
                let mut state = serializer.serialize_struct("NoteEnvelope", 2)?;
                state.serialize_field("ok", &true)?;
                state.serialize_field("note", note)?;
                state.end()
            }
            Self::Notes { notes } => {
                let mut state = serializer.serialize_struct("NoteEnvelope", 2)?;
                state.serialize_field("ok", &true)?;
                state.serialize_field("notes", notes)?;
                state.end()
            }
            Self::SearchHits { hits } => {
                let mut state = serializer.serialize_struct("NoteEnvelope", 2)?;
                state.serialize_field("ok", &true)?;
                state.serialize_field("hits", hits)?;
                state.end()
            }
            Self::ImageAsset { image } => {
                let mut state = serializer.serialize_struct("NoteEnvelope", 2)?;
                state.serialize_field("ok", &true)?;
                state.serialize_field("image", image)?;
                state.end()
            }
            Self::Updated {
                note,
                client_change_id,
            } => {
                let mut state = serializer.serialize_struct("NoteEnvelope", 3)?;
                state.serialize_field("ok", &true)?;
                state.serialize_field("note", note)?;
                state.serialize_field("clientChangeId", client_change_id)?;
                state.end()
            }
            Self::DateCounts { counts } => {
                let mut state = serializer.serialize_struct("NoteEnvelope", 2)?;
                state.serialize_field("ok", &true)?;
                state.serialize_field("counts", counts)?;
                state.end()
            }
            Self::DateChanged {
                note,
                previous_date,
                undo_token,
                client_change_id,
            } => {
                let mut state = serializer.serialize_struct("NoteEnvelope", 5)?;
                state.serialize_field("ok", &true)?;
                state.serialize_field("note", note)?;
                state.serialize_field("previousDate", previous_date)?;
                state.serialize_field("undoToken", undo_token)?;
                state.serialize_field("clientChangeId", client_change_id)?;
                state.end()
            }
            Self::DateUndoApplied {
                note,
                client_change_id,
            } => {
                let mut state = serializer.serialize_struct("NoteEnvelope", 3)?;
                state.serialize_field("ok", &true)?;
                state.serialize_field("note", note)?;
                state.serialize_field("clientChangeId", client_change_id)?;
                state.end()
            }
            Self::Tags { tags } => {
                let mut state = serializer.serialize_struct("NoteEnvelope", 2)?;
                state.serialize_field("ok", &true)?;
                state.serialize_field("tags", tags)?;
                state.end()
            }
            Self::Tag { tag } => {
                let mut state = serializer.serialize_struct("NoteEnvelope", 2)?;
                state.serialize_field("ok", &true)?;
                state.serialize_field("tag", tag)?;
                state.end()
            }
            Self::TagDeleted { deleted_tag_id } => {
                let mut state = serializer.serialize_struct("NoteEnvelope", 2)?;
                state.serialize_field("ok", &true)?;
                state.serialize_field("deletedTagId", deleted_tag_id)?;
                state.end()
            }
            Self::NoteDeleted {
                deleted_note_id,
                client_change_id,
            } => {
                let mut state = serializer.serialize_struct("NoteEnvelope", 3)?;
                state.serialize_field("ok", &true)?;
                state.serialize_field("deletedNoteId", deleted_note_id)?;
                state.serialize_field("clientChangeId", client_change_id)?;
                state.end()
            }
            Self::MetadataUpdated {
                note,
                client_change_id,
            } => {
                let mut state = serializer.serialize_struct("NoteEnvelope", 3)?;
                state.serialize_field("ok", &true)?;
                state.serialize_field("note", note)?;
                state.serialize_field("clientChangeId", client_change_id)?;
                state.end()
            }
            Self::Error { error } => {
                let mut state = serializer.serialize_struct("NoteEnvelope", 2)?;
                state.serialize_field("ok", &false)?;
                state.serialize_field("error", error)?;
                state.end()
            }
        }
    }
}

#[derive(Clone, Debug)]
pub struct IpcState {
    envelope: FoundationEnvelope,
}

impl IpcState {
    pub fn ready(status: FoundationStatus) -> Self {
        Self {
            envelope: FoundationEnvelope::Ready {
                status: status.into(),
            },
        }
    }

    pub fn ready_with_backup(status: FoundationStatus, backup_status: BackupHealth) -> Self {
        let mut status: FoundationStatusDto = status.into();
        status.backup_status = Some(backup_status.into());
        Self {
            envelope: FoundationEnvelope::Ready { status },
        }
    }

    pub fn failed(error: FoundationError) -> Self {
        let code = error.code();
        Self {
            envelope: FoundationEnvelope::Error {
                error: IpcErrorDto {
                    code,
                    message: error.safe_message().to_owned(),
                    recoverable: matches!(
                        code,
                        ErrorCode::KeyUnavailable
                            | ErrorCode::DatabaseWrongKey
                            | ErrorCode::DatabaseCorrupted
                            | ErrorCode::MigrationFailed
                            | ErrorCode::DataRootUnavailable
                            | ErrorCode::DiskFull
                    ),
                },
            },
        }
    }
}

pub fn handle_get_foundation_status(request: Value, state: &IpcState) -> FoundationEnvelope {
    let accepted = serde_json::from_value::<FoundationRequest>(request)
        .map(|request| request.protocol_version == IPC_PROTOCOL_VERSION)
        .unwrap_or(false);
    if !accepted {
        return FoundationEnvelope::Error {
            error: IpcErrorDto {
                code: ErrorCode::InternalError,
                message: "The desktop request was not accepted.".to_owned(),
                recoverable: false,
            },
        };
    }
    state.envelope.clone()
}

pub fn handle_create_note(request: Value, service: &NoteService<'_>) -> NoteEnvelope {
    let result = parse_request::<CreateNoteDto>(request).and_then(|request| {
        service.create_note(CreateNoteRequest {
            note_date: request.note_date,
            title: request.title,
            body_format: request.body_format,
            body_schema_version: request.body_schema_version,
            body_json: request.body_json,
        })
    });
    note_result(result)
}

pub fn handle_get_note(request: Value, service: &NoteService<'_>) -> NoteEnvelope {
    let result =
        parse_request::<GetNoteDto>(request).and_then(|request| service.get_note(&request.note_id));
    note_result(result)
}

pub fn handle_delete_note(request: Value, service: &NoteService<'_>) -> NoteEnvelope {
    let request = match parse_request::<DeleteNoteDto>(request) {
        Ok(request) => request,
        Err(error) => return note_error(error),
    };
    let deleted_note_id = request.note_id.clone();
    let client_change_id = request.client_change_id.clone();
    match service.delete_note(DeleteNoteRequest {
        note_id: request.note_id,
        base_revision: request.base_revision,
        client_change_id: request.client_change_id,
    }) {
        Ok(()) => NoteEnvelope::NoteDeleted {
            deleted_note_id,
            client_change_id,
        },
        Err(error) => note_error(error),
    }
}

pub fn handle_list_notes_for_date(request: Value, service: &NoteService<'_>) -> NoteEnvelope {
    match parse_request::<ListNotesForDateDto>(request)
        .and_then(|request| service.list_notes_for_date(&request.note_date))
    {
        Ok(notes) => NoteEnvelope::Notes {
            notes: notes.into_iter().map(Into::into).collect(),
        },
        Err(error) => note_error(error),
    }
}

pub fn handle_update_note_content(request: Value, service: &NoteService<'_>) -> NoteEnvelope {
    let request = match parse_request::<UpdateNoteContentDto>(request) {
        Ok(request) => request,
        Err(error) => return note_error(error),
    };
    let client_change_id = request.client_change_id.clone();
    match service
        .update_note_content(UpdateNoteContentRequest {
            note_id: request.note_id,
            title: request.title,
            body_format: request.body_format,
            body_schema_version: request.body_schema_version,
            body_json: request.body_json,
            base_revision: request.base_revision,
            client_change_id: request.client_change_id,
            content_hash: request.content_hash,
        })
        .and_then(TryInto::try_into)
    {
        Ok(note) => NoteEnvelope::Updated {
            note,
            client_change_id,
        },
        Err(error) => note_error(error),
    }
}

pub fn handle_list_note_counts_for_month(
    request: Value,
    service: &CalendarService<'_>,
) -> NoteEnvelope {
    match parse_request::<ListNoteCountsForMonthDto>(request)
        .and_then(|request| service.list_note_counts_for_month(&request.month))
    {
        Ok(counts) => NoteEnvelope::DateCounts {
            counts: counts.into_iter().map(Into::into).collect(),
        },
        Err(error) => note_error(error),
    }
}

pub fn handle_change_note_date(request: Value, service: &CalendarService<'_>) -> NoteEnvelope {
    let request = match parse_request::<ChangeNoteDateDto>(request) {
        Ok(request) => request,
        Err(error) => return note_error(error),
    };
    let client_change_id = request.client_change_id.clone();
    match service
        .change_note_date(ChangeNoteDateRequest {
            note_id: request.note_id,
            new_note_date: request.new_note_date,
            base_revision: request.base_revision,
            client_change_id: request.client_change_id,
        })
        .and_then(|receipt| {
            Ok((
                NoteDto::try_from(receipt.note)?,
                receipt.previous_date,
                DateChangeUndoTokenDto::from(receipt.undo_token),
            ))
        }) {
        Ok((note, previous_date, undo_token)) => NoteEnvelope::DateChanged {
            note,
            previous_date,
            undo_token,
            client_change_id,
        },
        Err(error) => note_error(error),
    }
}

pub fn handle_undo_note_date_change(request: Value, service: &CalendarService<'_>) -> NoteEnvelope {
    let request = match parse_request::<UndoNoteDateChangeDto>(request) {
        Ok(request) => request,
        Err(error) => return note_error(error),
    };
    let client_change_id = request.client_change_id.clone();
    match service
        .undo_note_date_change(UndoNoteDateChangeRequest {
            token_id: request.token_id,
            client_change_id: request.client_change_id,
        })
        .and_then(TryInto::try_into)
    {
        Ok(note) => NoteEnvelope::DateUndoApplied {
            note,
            client_change_id,
        },
        Err(error) => note_error(error),
    }
}

pub fn handle_list_tags(request: Value, service: &OrganizationService<'_>) -> NoteEnvelope {
    match parse_request::<ListTagsDto>(request).and_then(|_| service.list_tags()) {
        Ok(tags) => NoteEnvelope::Tags {
            tags: tags.into_iter().map(Into::into).collect(),
        },
        Err(error) => note_error(error),
    }
}

pub fn handle_create_tag(request: Value, service: &OrganizationService<'_>) -> NoteEnvelope {
    match parse_request::<CreateTagDto>(request)
        .and_then(|request| service.create_tag(&request.name))
    {
        Ok(tag) => NoteEnvelope::Tag { tag: tag.into() },
        Err(error) => note_error(error),
    }
}

pub fn handle_rename_tag(request: Value, service: &OrganizationService<'_>) -> NoteEnvelope {
    match parse_request::<RenameTagDto>(request).and_then(|request| {
        service.rename_tag(RenameTagRequest {
            tag_id: request.tag_id,
            name: request.name,
        })
    }) {
        Ok(tag) => NoteEnvelope::Tag { tag: tag.into() },
        Err(error) => note_error(error),
    }
}

pub fn handle_delete_tag(request: Value, service: &OrganizationService<'_>) -> NoteEnvelope {
    let request = match parse_request::<DeleteTagDto>(request) {
        Ok(request) => request,
        Err(error) => return note_error(error),
    };
    match service.delete_tag(&request.tag_id) {
        Ok(()) => NoteEnvelope::TagDeleted {
            deleted_tag_id: request.tag_id,
        },
        Err(error) => note_error(error),
    }
}

pub fn handle_list_tags_for_note(
    request: Value,
    service: &OrganizationService<'_>,
) -> NoteEnvelope {
    match parse_request::<ListTagsForNoteDto>(request)
        .and_then(|request| service.list_tags_for_note(&request.note_id))
    {
        Ok(tags) => NoteEnvelope::Tags {
            tags: tags.into_iter().map(Into::into).collect(),
        },
        Err(error) => note_error(error),
    }
}

pub fn handle_assign_tag(request: Value, service: &OrganizationService<'_>) -> NoteEnvelope {
    metadata_tag_result(request, service, true)
}

pub fn handle_remove_tag(request: Value, service: &OrganizationService<'_>) -> NoteEnvelope {
    metadata_tag_result(request, service, false)
}

fn metadata_tag_result(
    request: Value,
    service: &OrganizationService<'_>,
    assign: bool,
) -> NoteEnvelope {
    let request = match parse_request::<NoteTagMutationDto>(request) {
        Ok(request) => request,
        Err(error) => return note_error(error),
    };
    let client_change_id = request.client_change_id.clone();
    let mutation = NoteTagMutationRequest {
        note_id: request.note_id,
        tag_id: request.tag_id,
        base_revision: request.base_revision,
        client_change_id: request.client_change_id,
    };
    let result = if assign {
        service.assign_tag(mutation)
    } else {
        service.remove_tag(mutation)
    };
    match result.and_then(TryInto::try_into) {
        Ok(note) => NoteEnvelope::MetadataUpdated {
            note,
            client_change_id,
        },
        Err(error) => note_error(error),
    }
}

pub fn handle_set_note_pinned(request: Value, service: &OrganizationService<'_>) -> NoteEnvelope {
    let request = match parse_request::<SetNotePinnedDto>(request) {
        Ok(request) => request,
        Err(error) => return note_error(error),
    };
    let client_change_id = request.client_change_id.clone();
    match service
        .set_pinned(SetPinnedRequest {
            note_id: request.note_id,
            is_pinned: request.is_pinned,
            base_revision: request.base_revision,
            client_change_id: request.client_change_id,
        })
        .and_then(TryInto::try_into)
    {
        Ok(note) => NoteEnvelope::MetadataUpdated {
            note,
            client_change_id,
        },
        Err(error) => note_error(error),
    }
}

pub fn handle_list_recent_notes(request: Value, service: &OrganizationService<'_>) -> NoteEnvelope {
    match parse_request::<ListRecentNotesDto>(request).and_then(|_| service.list_recent()) {
        Ok(notes) => NoteEnvelope::Notes {
            notes: notes.into_iter().map(Into::into).collect(),
        },
        Err(error) => note_error(error),
    }
}

pub fn handle_search_notes(request: Value, service: &SearchService<'_>) -> NoteEnvelope {
    match parse_request::<SearchNotesDto>(request)
        .and_then(|request| service.search(&request.query))
    {
        Ok(hits) => NoteEnvelope::SearchHits {
            hits: hits.into_iter().map(Into::into).collect(),
        },
        Err(error) => note_error(error),
    }
}

pub fn handle_import_image_asset(request: Value, service: &ImageAssetService<'_>) -> NoteEnvelope {
    let result = parse_request::<ImportImageAssetDto>(request).and_then(|request| {
        let max_base64_length = MAX_IMAGE_INPUT_BYTES.div_ceil(3) * 4;
        if request.data_base64.is_empty() || request.data_base64.len() > max_base64_length {
            return Err(FoundationError::image_too_large());
        }
        let declared_format = match request.media_type.as_str() {
            "image/png" => ImageSourceFormat::Png,
            "image/jpeg" => ImageSourceFormat::Jpeg,
            "image/webp" => ImageSourceFormat::WebP,
            _ => return Err(FoundationError::image_rejected()),
        };
        let source = match request.source.as_str() {
            "editor_paste" => ImageInputSource::EditorPaste,
            "clipboard_screenshot" => ImageInputSource::ClipboardScreenshot,
            _ => return Err(FoundationError::image_rejected()),
        };
        let bytes = STANDARD
            .decode(request.data_base64.as_bytes())
            .map_err(|_| FoundationError::image_rejected())?;
        if bytes.len() > MAX_IMAGE_INPUT_BYTES {
            return Err(FoundationError::image_too_large());
        }
        service.import_image(ImportImageRequest {
            client_import_id: request.client_import_id,
            input: ImageInput {
                source,
                declared_format,
                bytes,
            },
        })
    });
    match result {
        Ok(image) => NoteEnvelope::ImageAsset {
            image: image.into(),
        },
        Err(error) => note_error(error),
    }
}

pub fn handle_read_image_asset(request: Value, service: &ImageAssetService<'_>) -> NoteEnvelope {
    match parse_request::<ReadImageAssetDto>(request)
        .and_then(|request| service.read_image(&request.note_id, &request.asset_id))
    {
        Ok(image) => NoteEnvelope::ImageAsset {
            image: image.into(),
        },
        Err(error) => note_error(error),
    }
}

pub fn handle_discard_image_asset(
    request: Value,
    service: &ImageAssetService<'_>,
) -> ActionEnvelope {
    match parse_request::<DiscardImageAssetDto>(request).and_then(|request| {
        service
            .discard_import(&request.client_import_id)
            .map(|_| ())
    }) {
        Ok(()) => ActionEnvelope::Done,
        Err(error) => action_error(error),
    }
}

pub fn handle_take_quick_capture<F>(request: Value, take: F) -> CaptureEnvelope
where
    F: FnOnce() -> Result<Option<CaptureSession>, FoundationError>,
{
    if let Err(error) = parse_request::<TakeQuickCaptureDto>(request) {
        return capture_error(error);
    }
    match take() {
        Ok(session) => CaptureEnvelope::Session {
            session: session.map(Into::into),
        },
        Err(error) => capture_error(error),
    }
}

pub fn handle_finish_quick_capture<F>(request: Value, finish: F) -> ActionEnvelope
where
    F: FnOnce(&str) -> Result<(), FoundationError>,
{
    match parse_request::<FinishQuickCaptureDto>(request)
        .and_then(|request| finish(&request.session_id))
    {
        Ok(()) => ActionEnvelope::Done,
        Err(error) => action_error(error),
    }
}

pub fn handle_get_shortcut_config<F>(request: Value, get: F) -> ShortcutEnvelope
where
    F: FnOnce() -> Result<ShortcutRuntimeStatus, FoundationError>,
{
    if let Err(error) = parse_request::<GetShortcutConfigDto>(request) {
        return shortcut_error(error);
    }
    match get() {
        Ok(status) => ShortcutEnvelope::Status {
            status: status.into(),
        },
        Err(error) => shortcut_error(error),
    }
}

pub fn handle_set_shortcut_config<F>(request: Value, set: F) -> ShortcutEnvelope
where
    F: FnOnce(&str) -> Result<ShortcutRuntimeStatus, FoundationError>,
{
    let request = match parse_request::<SetShortcutConfigDto>(request) {
        Ok(request) => request,
        Err(error) => return shortcut_error(error),
    };
    match set(&request.shortcut) {
        Ok(status) => ShortcutEnvelope::Status {
            status: status.into(),
        },
        Err(error) => shortcut_error(error),
    }
}

pub fn handle_get_backup_status<F>(request: Value, get: F) -> BackupEnvelope
where
    F: FnOnce() -> Result<BackupHealth, FoundationError>,
{
    if let Err(error) = parse_request::<GetBackupStatusDto>(request) {
        return backup_error(error);
    }
    match get() {
        Ok(status) => BackupEnvelope::Status {
            status: status.into(),
        },
        Err(error) => backup_error(error),
    }
}

pub fn note_error(error: FoundationError) -> NoteEnvelope {
    NoteEnvelope::Error {
        error: error_dto(error),
    }
}

fn capture_error(error: FoundationError) -> CaptureEnvelope {
    CaptureEnvelope::Error {
        error: error_dto(error),
    }
}

pub fn action_error(error: FoundationError) -> ActionEnvelope {
    ActionEnvelope::Error {
        error: error_dto(error),
    }
}

fn shortcut_error(error: FoundationError) -> ShortcutEnvelope {
    ShortcutEnvelope::Error {
        error: error_dto(error),
    }
}

fn backup_error(error: FoundationError) -> BackupEnvelope {
    BackupEnvelope::Error {
        error: error_dto(error),
    }
}

fn note_result(result: Result<Note, FoundationError>) -> NoteEnvelope {
    match result.and_then(TryInto::try_into) {
        Ok(note) => NoteEnvelope::Note { note },
        Err(error) => note_error(error),
    }
}

fn parse_request<T>(request: Value) -> Result<T, FoundationError>
where
    T: for<'de> Deserialize<'de> + ProtocolRequest,
{
    let request =
        serde_json::from_value::<T>(request).map_err(|_| FoundationError::validation_failed())?;
    if request.protocol_version() != IPC_PROTOCOL_VERSION {
        return Err(FoundationError::validation_failed());
    }
    Ok(request)
}

trait ProtocolRequest {
    fn protocol_version(&self) -> u16;
}

macro_rules! protocol_request {
    ($($request:ty),+ $(,)?) => {
        $(impl ProtocolRequest for $request {
            fn protocol_version(&self) -> u16 {
                self.protocol_version
            }
        })+
    };
}

protocol_request!(
    CreateNoteDto,
    GetNoteDto,
    DeleteNoteDto,
    ListNotesForDateDto,
    UpdateNoteContentDto,
    ListNoteCountsForMonthDto,
    ChangeNoteDateDto,
    UndoNoteDateChangeDto,
    ListTagsDto,
    CreateTagDto,
    RenameTagDto,
    DeleteTagDto,
    ListTagsForNoteDto,
    NoteTagMutationDto,
    SetNotePinnedDto,
    ListRecentNotesDto,
    SearchNotesDto,
    ImportImageAssetDto,
    ReadImageAssetDto,
    DiscardImageAssetDto,
    TakeQuickCaptureDto,
    FinishQuickCaptureDto,
    GetShortcutConfigDto,
    SetShortcutConfigDto,
    GetBackupStatusDto
);

fn error_dto(error: FoundationError) -> IpcErrorDto {
    let code = error.code();
    IpcErrorDto {
        code,
        message: error.safe_message().to_owned(),
        recoverable: matches!(
            code,
            ErrorCode::KeyUnavailable
                | ErrorCode::DatabaseWrongKey
                | ErrorCode::DatabaseCorrupted
                | ErrorCode::MigrationFailed
                | ErrorCode::DataRootUnavailable
                | ErrorCode::DiskFull
                | ErrorCode::NoteNotFound
                | ErrorCode::RevisionConflict
                | ErrorCode::UndoExpired
                | ErrorCode::TagNotFound
                | ErrorCode::TagNameConflict
                | ErrorCode::ImageRejected
                | ErrorCode::ImageTooLarge
                | ErrorCode::AssetNotFound
                | ErrorCode::AssetCorrupted
                | ErrorCode::ShortcutInvalid
                | ErrorCode::ShortcutConflict
                | ErrorCode::CaptureSessionMismatch
                | ErrorCode::BackupBusy
                | ErrorCode::BackupFailed
                | ErrorCode::BackupCorrupted
                | ErrorCode::BackupUnsupported
        ),
    }
}

impl fmt::Display for IpcErrorDto {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}
