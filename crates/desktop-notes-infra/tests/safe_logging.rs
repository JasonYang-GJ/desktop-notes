use std::{fs, path::PathBuf, time::SystemTime};

use desktop_notes_core::{SafeLogEvent, SafeLogKind, SafeLogger};
use desktop_notes_infra::SafeJsonlLogger;

fn isolated_log() -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir()
        .join(format!(
            "desktop-notes-b01-log-{}-{nonce}",
            std::process::id()
        ))
        .join("events.jsonl")
}

#[test]
fn invalid_operation_id_cannot_inject_content_into_structured_log() {
    let path = isolated_log();
    let logger = SafeJsonlLogger::new(path.clone()).unwrap();
    let forbidden = "SECRET_NOTE_BODY_MARKER";

    logger.event(SafeLogEvent {
        operation_id: forbidden.to_owned(),
        module: "foundation",
        kind: SafeLogKind::FoundationReady,
        error_code: None,
        duration_ms: 3,
        schema_version: Some(1),
        os_capability: Some("dpapi_current_user"),
    });

    let log = fs::read_to_string(&path).unwrap();
    assert!(!log.contains(forbidden));
    assert!(log.contains("invalid-operation-id"));
    fs::remove_dir_all(path.parent().unwrap()).unwrap();
}
