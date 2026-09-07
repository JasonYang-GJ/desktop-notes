use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::PathBuf,
    sync::Mutex,
};

use desktop_notes_core::{ErrorCode, FoundationError, SafeLogEvent, SafeLogger};

pub struct SafeJsonlLogger {
    path: PathBuf,
    write_lock: Mutex<()>,
}

impl SafeJsonlLogger {
    pub fn new(path: PathBuf) -> Result<Self, FoundationError> {
        let parent = path.parent().ok_or_else(log_unavailable)?;
        fs::create_dir_all(parent).map_err(|_| log_unavailable())?;
        OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .map_err(|_| log_unavailable())?;
        Ok(Self {
            path,
            write_lock: Mutex::new(()),
        })
    }
}

impl SafeLogger for SafeJsonlLogger {
    fn event(&self, mut event: SafeLogEvent) {
        event.operation_id = sanitize_operation_id(&event.operation_id);
        event.module = match event.module {
            "foundation" => "foundation",
            "ipc" => "ipc",
            "lifecycle" => "lifecycle",
            "platform" => "platform",
            _ => "unknown",
        };
        event.os_capability = match event.os_capability {
            Some("dpapi_current_user") => Some("dpapi_current_user"),
            Some("webview2") => Some("webview2"),
            Some(_) => Some("unknown"),
            None => None,
        };

        let Ok(_guard) = self.write_lock.lock() else {
            return;
        };
        let Ok(mut file) = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
        else {
            return;
        };
        if serde_json::to_writer(&mut file, &event).is_ok() {
            let _ = file.write_all(b"\n");
            let _ = file.flush();
        }
    }
}

fn sanitize_operation_id(operation_id: &str) -> String {
    if !operation_id.is_empty()
        && operation_id.len() <= 64
        && operation_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
    {
        operation_id.to_owned()
    } else {
        "invalid-operation-id".to_owned()
    }
}

fn log_unavailable() -> FoundationError {
    FoundationError::new(
        ErrorCode::DataRootUnavailable,
        "The structured log directory is unavailable.",
    )
}
