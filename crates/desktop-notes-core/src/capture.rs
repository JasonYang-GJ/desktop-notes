use std::sync::Mutex;

use crate::FoundationError;

pub const DEFAULT_QUICK_CAPTURE_SHORTCUT: &str = "Ctrl+Alt+N";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ShortcutSpec(String);

impl ShortcutSpec {
    pub fn parse(value: &str) -> Result<Self, FoundationError> {
        let mut ctrl = false;
        let mut alt = false;
        let mut shift = false;
        let mut super_key = false;
        let mut key: Option<String> = None;

        for raw in value.split('+') {
            let token = raw.trim();
            if token.is_empty() {
                return Err(FoundationError::shortcut_invalid());
            }
            match token.to_ascii_lowercase().as_str() {
                "ctrl" | "control" => set_once(&mut ctrl)?,
                "alt" => set_once(&mut alt)?,
                "shift" => set_once(&mut shift)?,
                "super" | "win" | "meta" => set_once(&mut super_key)?,
                _ if key.is_none() => key = Some(normalize_key(token)?),
                _ => return Err(FoundationError::shortcut_invalid()),
            }
        }

        let modifier_count = [ctrl, alt, shift, super_key]
            .into_iter()
            .filter(|present| *present)
            .count();
        let key = key.ok_or_else(FoundationError::shortcut_invalid)?;
        if modifier_count < 2 || (!ctrl && !alt) {
            return Err(FoundationError::shortcut_invalid());
        }

        let mut parts = Vec::with_capacity(modifier_count + 1);
        if ctrl {
            parts.push("Ctrl".to_owned());
        }
        if alt {
            parts.push("Alt".to_owned());
        }
        if shift {
            parts.push("Shift".to_owned());
        }
        if super_key {
            parts.push("Super".to_owned());
        }
        parts.push(key);
        Ok(Self(parts.join("+")))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

fn set_once(slot: &mut bool) -> Result<(), FoundationError> {
    if *slot {
        return Err(FoundationError::shortcut_invalid());
    }
    *slot = true;
    Ok(())
}

fn normalize_key(value: &str) -> Result<String, FoundationError> {
    let upper = value.to_ascii_uppercase();
    let bytes = upper.as_bytes();
    if bytes.len() == 1 && bytes[0].is_ascii_alphanumeric() {
        return Ok(upper);
    }
    if let Some(number) = upper
        .strip_prefix('F')
        .and_then(|value| value.parse::<u8>().ok())
        && (1..=12).contains(&number)
    {
        return Ok(format!("F{number}"));
    }
    Err(FoundationError::shortcut_invalid())
}

pub trait HotkeyRegistrar {
    fn register(&self, shortcut: &ShortcutSpec) -> Result<(), FoundationError>;
    fn unregister(&self, shortcut: &ShortcutSpec) -> Result<(), FoundationError>;
}

pub trait ShortcutSettingsStore {
    fn load_quick_capture_shortcut(&self) -> Result<Option<String>, FoundationError>;
    fn save_quick_capture_shortcut(
        &self,
        shortcut: &ShortcutSpec,
        updated_at_ms: i64,
    ) -> Result<(), FoundationError>;
}

pub fn replace_shortcut_atomically(
    registrar: &dyn HotkeyRegistrar,
    settings: &dyn ShortcutSettingsStore,
    current: Option<&ShortcutSpec>,
    requested: &str,
    updated_at_ms: i64,
) -> Result<ShortcutSpec, FoundationError> {
    if updated_at_ms < 0 {
        return Err(FoundationError::validation_failed());
    }
    let next = ShortcutSpec::parse(requested)?;
    if current == Some(&next) {
        settings.save_quick_capture_shortcut(&next, updated_at_ms)?;
        return Ok(next);
    }

    registrar.register(&next)?;
    if let Some(previous) = current
        && registrar.unregister(previous).is_err()
    {
        let _ = registrar.unregister(&next);
        return Err(FoundationError::shortcut_conflict());
    }

    if let Err(error) = settings.save_quick_capture_shortcut(&next, updated_at_ms) {
        if let Some(previous) = current {
            let _ = registrar.register(previous);
        }
        let _ = registrar.unregister(&next);
        return Err(error);
    }
    Ok(next)
}

pub enum CapturedClipboard {
    Text(String),
    ImagePng(Vec<u8>),
    TextAndImage { text: String, png: Vec<u8> },
    Empty,
    Unsupported,
    Busy,
    Failed,
}

impl CapturedClipboard {
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Text(_) => "text",
            Self::ImagePng(_) => "image",
            Self::TextAndImage { .. } => "text_and_image",
            Self::Empty => "empty",
            Self::Unsupported => "unsupported",
            Self::Busy => "busy",
            Self::Failed => "failed",
        }
    }
}

pub struct ClipboardRead {
    pub content: CapturedClipboard,
    pub attempts: u8,
}

pub trait ClipboardReader: Send + Sync {
    fn read_once(&self) -> ClipboardRead;
}

pub struct CaptureSession {
    pub id: String,
    pub clipboard: CapturedClipboard,
    pub acquisition_attempts: u8,
}

pub enum CaptureTrigger {
    Started,
    AlreadyActive,
}

enum CaptureState {
    Idle,
    Starting,
    Pending(CaptureSession),
    Delivered(String),
}

pub struct CaptureCoordinator {
    state: Mutex<CaptureState>,
}

impl Default for CaptureCoordinator {
    fn default() -> Self {
        Self {
            state: Mutex::new(CaptureState::Idle),
        }
    }
}

impl CaptureCoordinator {
    pub fn trigger(
        &self,
        session_id: String,
        clipboard: &dyn ClipboardReader,
    ) -> Result<CaptureTrigger, FoundationError> {
        let mut state = self.state.lock().map_err(|_| internal_error())?;
        if !matches!(*state, CaptureState::Idle) {
            return Ok(CaptureTrigger::AlreadyActive);
        }
        *state = CaptureState::Starting;
        drop(state);

        let read = clipboard.read_once();
        let mut state = self.state.lock().map_err(|_| internal_error())?;
        *state = CaptureState::Pending(CaptureSession {
            id: session_id,
            clipboard: read.content,
            acquisition_attempts: read.attempts,
        });
        Ok(CaptureTrigger::Started)
    }

    pub fn take_pending(&self) -> Result<Option<CaptureSession>, FoundationError> {
        let mut state = self.state.lock().map_err(|_| internal_error())?;
        let pending = std::mem::replace(&mut *state, CaptureState::Starting);
        match pending {
            CaptureState::Pending(session) => {
                *state = CaptureState::Delivered(session.id.clone());
                Ok(Some(session))
            }
            other => {
                *state = other;
                Ok(None)
            }
        }
    }

    pub fn finish(&self, session_id: &str) -> Result<(), FoundationError> {
        let mut state = self.state.lock().map_err(|_| internal_error())?;
        let matches = match &*state {
            CaptureState::Pending(session) => session.id == session_id,
            CaptureState::Delivered(id) => id == session_id,
            CaptureState::Idle | CaptureState::Starting => false,
        };
        if !matches {
            return Err(FoundationError::capture_session_mismatch());
        }
        *state = CaptureState::Idle;
        Ok(())
    }

    pub fn is_active(&self) -> bool {
        self.state
            .lock()
            .map(|state| !matches!(*state, CaptureState::Idle))
            .unwrap_or(true)
    }
}

fn internal_error() -> FoundationError {
    FoundationError::new(
        crate::ErrorCode::InternalError,
        "The quick-capture state is unavailable.",
    )
}
