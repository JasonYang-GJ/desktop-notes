use desktop_notes_core::{ErrorCode, FoundationError};
use desktop_notes_desktop::ipc::{ALLOWED_COMMANDS, handle_get_backup_status};
use desktop_notes_infra::{BackupHealth, BackupHealthState};
use serde_json::json;

#[test]
fn backup_health_is_typed_minimal_and_path_free() {
    let response = serde_json::to_value(handle_get_backup_status(
        json!({ "protocolVersion": 1 }),
        || {
            Ok(BackupHealth {
                state: BackupHealthState::Healthy,
                last_success_at_ms: Some(1_789_750_800_000),
                last_success_local_day: Some("2026-09-07".to_owned()),
                valid_generation_count: 2,
                last_error_code: None,
            })
        },
    ))
    .unwrap();

    assert_eq!(response["ok"], true);
    assert_eq!(response["status"]["state"], "healthy");
    assert_eq!(response["status"]["validGenerationCount"], 2);
    assert!(!response.to_string().contains("storagePath"));
    assert!(ALLOWED_COMMANDS.contains(&"get_backup_status"));
}

#[test]
fn malformed_or_failed_status_requests_are_classified() {
    let malformed = serde_json::to_value(handle_get_backup_status(
        json!({ "protocolVersion": 1, "unexpected": true }),
        || Ok(BackupHealth::default()),
    ))
    .unwrap();
    assert_eq!(malformed["error"]["code"], "VALIDATION_FAILED");

    let failed = serde_json::to_value(handle_get_backup_status(
        json!({ "protocolVersion": 1 }),
        || Err(FoundationError::backup_failed()),
    ))
    .unwrap();
    assert_eq!(failed["error"]["code"], "BACKUP_FAILED");
    assert_eq!(failed["error"]["recoverable"], true);
    assert_ne!(ErrorCode::BackupFailed, ErrorCode::InternalError);
}
