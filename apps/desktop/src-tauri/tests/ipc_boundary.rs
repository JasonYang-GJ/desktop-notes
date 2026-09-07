use desktop_notes_core::{DatabaseState, ErrorCode, FoundationError, FoundationStatus};
use desktop_notes_desktop::ipc::{
    ALLOWED_COMMANDS, FoundationEnvelope, IpcState, handle_get_foundation_status,
};
use serde_json::json;
use std::{fs, path::PathBuf};

fn ready_state() -> IpcState {
    IpcState::ready(FoundationStatus {
        ready: true,
        view: "today_normal_empty",
        database_state: DatabaseState::Fresh,
        schema_version: 1,
        encryption: "sqlcipher",
    })
}

#[test]
fn main_window_capability_grants_only_the_narrow_destroy_permission_beyond_core_defaults() {
    let capability_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("capabilities")
        .join("default.json");
    let capability: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(capability_path).unwrap()).unwrap();

    assert_eq!(capability["windows"], json!(["main"]));
    assert_eq!(
        capability["permissions"],
        json!(["core:default", "core:window:allow-destroy"])
    );
}

#[test]
fn command_allowlist_contains_only_the_approved_b01_through_b10_boundaries() {
    assert_eq!(
        ALLOWED_COMMANDS,
        [
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
        ]
    );
}

#[test]
fn valid_request_returns_a_versioned_typed_envelope() {
    let envelope = handle_get_foundation_status(json!({ "protocolVersion": 1 }), &ready_state());
    let encoded = serde_json::to_value(envelope).unwrap();

    assert_eq!(encoded["ok"], true);
    assert_eq!(encoded["status"]["protocolVersion"], 1);
    assert_eq!(encoded["status"]["view"], "today_normal_empty");
    assert!(encoded.get("error").is_none());
}

#[test]
fn malformed_or_unknown_dto_is_mapped_without_parser_details() {
    for invalid in [
        json!({}),
        json!({ "protocolVersion": "one" }),
        json!({ "protocolVersion": 2 }),
        json!({ "protocolVersion": 1, "extra": "not-allowed" }),
    ] {
        let envelope = handle_get_foundation_status(invalid, &ready_state());
        let FoundationEnvelope::Error { error } = envelope else {
            panic!("invalid DTO unexpectedly succeeded");
        };
        assert_eq!(error.code, ErrorCode::InternalError);
        assert_eq!(error.message, "The desktop request was not accepted.");
    }
}

#[test]
fn foundation_error_code_and_safe_message_cross_the_boundary() {
    let state = IpcState::failed(FoundationError::key_unavailable());
    let envelope = handle_get_foundation_status(json!({ "protocolVersion": 1 }), &state);
    let FoundationEnvelope::Error { error } = envelope else {
        panic!("failed foundation unexpectedly succeeded");
    };

    assert_eq!(error.code, ErrorCode::KeyUnavailable);
    assert!(error.recoverable);
    assert!(!error.message.contains("keyring.json"));
}
