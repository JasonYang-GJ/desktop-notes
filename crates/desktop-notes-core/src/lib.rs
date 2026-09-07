//! Platform-independent Desktop Notes domain and application boundary.

mod application;
mod assets;
mod calendar;
mod capture;
mod error;
mod note_application;
mod notes;
mod organization;
mod organization_application;
mod ports;
mod search_application;
mod secrets;

pub use application::{DatabaseState, FoundationService, FoundationStatus};
pub use assets::{
    ASSET_GC_GRACE_MS, AssetRecord, AssetStartupRepair, DisplayImage, ImageAssetService,
    ImageInput, ImageInputSource, ImageSourceFormat, ImportImageRequest, PersistedImage,
    StagedAssetImport, gc_error_code,
};
pub use calendar::{
    CalendarService, ChangeNoteDateRequest, DATE_CHANGE_UNDO_WINDOW_MS, DateChangeReceipt,
    DateChangeUndoToken, DateCount, DateUndoRecord, NoteDateUpdate, UndoNoteDateChangeRequest,
};
pub use capture::{
    CaptureCoordinator, CaptureSession, CaptureTrigger, CapturedClipboard, ClipboardRead,
    ClipboardReader, DEFAULT_QUICK_CAPTURE_SHORTCUT, HotkeyRegistrar, ShortcutSettingsStore,
    ShortcutSpec, replace_shortcut_atomically,
};
pub use error::{ErrorCode, FoundationError};
pub use note_application::NoteService;
pub use notes::{
    BODY_FORMAT, BODY_SCHEMA_VERSION, CreateNoteRequest, DeleteNoteRequest, MAX_BODY_JSON_BYTES,
    MAX_TITLE_CHARS, NewNote, Note, NoteDeletion, NoteSummary, NoteUpdate, SearchHit,
    SearchMatchTag, UpdateNoteContentRequest, calculate_content_hash_hex,
    extract_image_asset_occurrences, validate_note_date, validate_note_id,
};
pub use organization::{
    AssignTagRequest, MAX_TAG_NAME_BYTES, MAX_TAG_NAME_CHARS, NewTag, NotePinUpdate,
    NoteTagMutationRequest, NoteTagUpdate, PreparedTagName, RenameTagRequest, SetPinnedRequest,
    Tag, TagRename, prepare_tag_name,
};
pub use organization_application::OrganizationService;
pub use ports::{
    AssetFileStore, AssetIdGenerator, AssetStore, Clock, DatabaseStatus, DateUndoStore,
    EncryptedStore, KeyDeriver, KeyProtector, NoteIdGenerator, NoteRepository,
    OrganizationRepository, SafeLogEvent, SafeLogKind, SafeLogger, SearchRepository,
};
pub use search_application::{MAX_SEARCH_QUERY_BYTES, MAX_SEARCH_QUERY_CHARS, SearchService};
pub use secrets::{RootKey, SecretKey};
