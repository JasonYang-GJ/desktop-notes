use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ErrorCode {
    KeyUnavailable,
    DatabaseWrongKey,
    DatabaseCorrupted,
    DatabaseSchemaTooNew,
    MigrationFailed,
    DataRootUnavailable,
    DiskFull,
    WebviewUnavailable,
    ValidationFailed,
    NoteNotFound,
    TagNotFound,
    TagNameConflict,
    RevisionConflict,
    UndoExpired,
    ImageRejected,
    ImageTooLarge,
    AssetNotFound,
    AssetCorrupted,
    ShortcutInvalid,
    ShortcutConflict,
    CaptureSessionMismatch,
    BackupBusy,
    BackupFailed,
    BackupCorrupted,
    BackupUnsupported,
    InternalError,
}

#[derive(Debug, Error)]
#[error("{message}")]
pub struct FoundationError {
    code: ErrorCode,
    message: &'static str,
}

impl FoundationError {
    pub const fn new(code: ErrorCode, message: &'static str) -> Self {
        Self { code, message }
    }

    pub const fn code(&self) -> ErrorCode {
        self.code
    }

    pub const fn safe_message(&self) -> &'static str {
        self.message
    }

    pub const fn key_unavailable() -> Self {
        Self::new(
            ErrorCode::KeyUnavailable,
            "The local encryption key is unavailable. Existing data was not replaced.",
        )
    }

    pub const fn validation_failed() -> Self {
        Self::new(
            ErrorCode::ValidationFailed,
            "The note request did not match the supported format.",
        )
    }

    pub const fn note_not_found() -> Self {
        Self::new(ErrorCode::NoteNotFound, "The requested note was not found.")
    }

    pub const fn tag_not_found() -> Self {
        Self::new(ErrorCode::TagNotFound, "The requested tag was not found.")
    }

    pub const fn tag_name_conflict() -> Self {
        Self::new(
            ErrorCode::TagNameConflict,
            "A tag with the same normalized name already exists.",
        )
    }

    pub const fn revision_conflict() -> Self {
        Self::new(
            ErrorCode::RevisionConflict,
            "The note changed before this save completed. Reload the latest revision.",
        )
    }

    pub const fn undo_expired() -> Self {
        Self::new(
            ErrorCode::UndoExpired,
            "The date change can no longer be undone safely.",
        )
    }

    pub const fn image_rejected() -> Self {
        Self::new(
            ErrorCode::ImageRejected,
            "The pasted image is not a supported PNG, JPEG, or WebP image.",
        )
    }

    pub const fn image_too_large() -> Self {
        Self::new(
            ErrorCode::ImageTooLarge,
            "The pasted image exceeds the safe image size limit.",
        )
    }

    pub const fn asset_not_found() -> Self {
        Self::new(
            ErrorCode::AssetNotFound,
            "The encrypted image is not available to this Note.",
        )
    }

    pub const fn asset_corrupted() -> Self {
        Self::new(
            ErrorCode::AssetCorrupted,
            "The encrypted image failed its integrity check.",
        )
    }

    pub const fn shortcut_invalid() -> Self {
        Self::new(
            ErrorCode::ShortcutInvalid,
            "Use at least two modifiers and one supported letter, number, or function key.",
        )
    }

    pub const fn shortcut_conflict() -> Self {
        Self::new(
            ErrorCode::ShortcutConflict,
            "That shortcut is unavailable. The previous shortcut is still active.",
        )
    }

    pub const fn capture_session_mismatch() -> Self {
        Self::new(
            ErrorCode::CaptureSessionMismatch,
            "The quick-capture session is no longer active.",
        )
    }

    pub const fn backup_busy() -> Self {
        Self::new(
            ErrorCode::BackupBusy,
            "An automatic backup is already running.",
        )
    }

    pub const fn backup_failed() -> Self {
        Self::new(
            ErrorCode::BackupFailed,
            "The automatic backup could not be completed. Existing Notes and backups were not replaced.",
        )
    }

    pub const fn backup_corrupted() -> Self {
        Self::new(
            ErrorCode::BackupCorrupted,
            "The backup package failed its integrity check.",
        )
    }

    pub const fn backup_unsupported() -> Self {
        Self::new(
            ErrorCode::BackupUnsupported,
            "The backup package version or protection profile is not supported.",
        )
    }
}
