use std::sync::atomic::{AtomicUsize, Ordering};

use desktop_notes_core::{CaptureSession, CapturedClipboard, ErrorCode, FoundationError};
use desktop_notes_desktop::ipc::{
    ALLOWED_COMMANDS, ShortcutRuntimeStatus, handle_finish_quick_capture,
    handle_get_shortcut_config, handle_set_shortcut_config, handle_take_quick_capture,
};
use serde_json::json;

const SESSION_ID: &str = "83000000-0000-4000-8000-000000000001";

#[test]
fn capture_ipc_returns_text_and_image_from_one_memory_session_without_paths() {
    let response = serde_json::to_value(handle_take_quick_capture(
        json!({ "protocolVersion": 1 }),
        || {
            Ok(Some(CaptureSession {
                id: SESSION_ID.to_owned(),
                clipboard: CapturedClipboard::TextAndImage {
                    text: "synthetic clipboard text".to_owned(),
                    png: vec![1, 2, 3, 4],
                },
                acquisition_attempts: 1,
            }))
        },
    ))
    .unwrap();

    assert_eq!(response["ok"], true);
    assert_eq!(response["session"]["id"], SESSION_ID);
    assert_eq!(response["session"]["contentType"], "text_and_image");
    assert_eq!(response["session"]["text"], "synthetic clipboard text");
    assert_eq!(response["session"]["dataBase64"], "AQIDBA==");
    assert_eq!(response["session"]["mediaType"], "image/png");
    assert!(!response.to_string().contains("storage_relpath"));
    assert!(!response.to_string().contains("absolutePath"));
}

#[test]
fn malformed_take_request_cannot_consume_the_pending_session() {
    let calls = AtomicUsize::new(0);
    let response = serde_json::to_value(handle_take_quick_capture(
        json!({ "protocolVersion": 1, "unexpected": true }),
        || {
            calls.fetch_add(1, Ordering::SeqCst);
            Ok(None)
        },
    ))
    .unwrap();
    assert_eq!(response["ok"], false);
    assert_eq!(response["error"]["code"], "VALIDATION_FAILED");
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[test]
fn finish_and_shortcut_commands_are_strictly_typed() {
    let finish = serde_json::to_value(handle_finish_quick_capture(
        json!({ "protocolVersion": 1, "sessionId": SESSION_ID }),
        |id| {
            assert_eq!(id, SESSION_ID);
            Ok(())
        },
    ))
    .unwrap();
    assert_eq!(finish, json!({ "ok": true }));

    let status = ShortcutRuntimeStatus {
        configured_shortcut: "Ctrl+Alt+N".to_owned(),
        active_shortcut: Some("Ctrl+Alt+N".to_owned()),
        registration_error: None,
    };
    let get = serde_json::to_value(handle_get_shortcut_config(
        json!({ "protocolVersion": 1 }),
        || Ok(status.clone()),
    ))
    .unwrap();
    assert_eq!(get["status"]["activeShortcut"], "Ctrl+Alt+N");

    let set = serde_json::to_value(handle_set_shortcut_config(
        json!({ "protocolVersion": 1, "shortcut": "Ctrl+Alt+Q" }),
        |requested| {
            assert_eq!(requested, "Ctrl+Alt+Q");
            Ok(ShortcutRuntimeStatus {
                configured_shortcut: requested.to_owned(),
                active_shortcut: Some(requested.to_owned()),
                registration_error: None,
            })
        },
    ))
    .unwrap();
    assert_eq!(set["status"]["configuredShortcut"], "Ctrl+Alt+Q");

    let conflict = serde_json::to_value(handle_set_shortcut_config(
        json!({ "protocolVersion": 1, "shortcut": "Ctrl+Alt+P" }),
        |_| Err(FoundationError::shortcut_conflict()),
    ))
    .unwrap();
    assert_eq!(conflict["error"]["code"], "SHORTCUT_CONFLICT");
    assert_eq!(conflict["error"]["recoverable"], true);
}

#[test]
fn b08_commands_are_present_in_the_custom_allowlist() {
    for command in [
        "discard_image_asset",
        "take_quick_capture",
        "finish_quick_capture",
        "get_shortcut_config",
        "set_shortcut_config",
    ] {
        assert!(ALLOWED_COMMANDS.contains(&command));
    }
    assert_ne!(ErrorCode::ShortcutConflict, ErrorCode::InternalError);
}
