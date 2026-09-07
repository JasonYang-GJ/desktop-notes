use std::sync::{
    Mutex,
    atomic::{AtomicUsize, Ordering},
};

use desktop_notes_core::{
    CaptureCoordinator, CaptureTrigger, CapturedClipboard, ClipboardRead, ClipboardReader,
    DEFAULT_QUICK_CAPTURE_SHORTCUT, ErrorCode, FoundationError, HotkeyRegistrar,
    ShortcutSettingsStore, ShortcutSpec, replace_shortcut_atomically,
};

#[derive(Default)]
struct FakeHotkeys {
    calls: Mutex<Vec<String>>,
    fail_register: Mutex<Option<String>>,
    fail_unregister: Mutex<Option<String>>,
}

impl HotkeyRegistrar for FakeHotkeys {
    fn register(&self, shortcut: &ShortcutSpec) -> Result<(), FoundationError> {
        self.calls
            .lock()
            .unwrap()
            .push(format!("register:{}", shortcut.as_str()));
        if self.fail_register.lock().unwrap().as_deref() == Some(shortcut.as_str()) {
            return Err(FoundationError::shortcut_conflict());
        }
        Ok(())
    }

    fn unregister(&self, shortcut: &ShortcutSpec) -> Result<(), FoundationError> {
        self.calls
            .lock()
            .unwrap()
            .push(format!("unregister:{}", shortcut.as_str()));
        if self.fail_unregister.lock().unwrap().as_deref() == Some(shortcut.as_str()) {
            return Err(FoundationError::shortcut_conflict());
        }
        Ok(())
    }
}

#[derive(Default)]
struct FakeSettings {
    value: Mutex<Option<String>>,
    fail_save: Mutex<bool>,
}

impl ShortcutSettingsStore for FakeSettings {
    fn load_quick_capture_shortcut(&self) -> Result<Option<String>, FoundationError> {
        Ok(self.value.lock().unwrap().clone())
    }

    fn save_quick_capture_shortcut(
        &self,
        shortcut: &ShortcutSpec,
        _updated_at_ms: i64,
    ) -> Result<(), FoundationError> {
        if *self.fail_save.lock().unwrap() {
            return Err(FoundationError::new(
                ErrorCode::InternalError,
                "Synthetic settings failure.",
            ));
        }
        *self.value.lock().unwrap() = Some(shortcut.as_str().to_owned());
        Ok(())
    }
}

struct CountingClipboard {
    calls: AtomicUsize,
}

impl ClipboardReader for CountingClipboard {
    fn read_once(&self) -> ClipboardRead {
        self.calls.fetch_add(1, Ordering::SeqCst);
        ClipboardRead {
            content: CapturedClipboard::Text("synthetic capture".to_owned()),
            attempts: 1,
        }
    }
}

#[test]
fn default_shortcut_is_valid_and_canonical() {
    assert_eq!(
        ShortcutSpec::parse(DEFAULT_QUICK_CAPTURE_SHORTCUT)
            .unwrap()
            .as_str(),
        "Ctrl+Alt+N"
    );
    assert_eq!(
        ShortcutSpec::parse("alt + ctrl + shift + f12")
            .unwrap()
            .as_str(),
        "Ctrl+Alt+Shift+F12"
    );
}

#[test]
fn unsafe_or_ambiguous_shortcuts_are_rejected() {
    for invalid in ["N", "Ctrl+N", "Ctrl+Alt", "Ctrl+Ctrl+N", "Ctrl+Alt+Delete"] {
        assert_eq!(
            ShortcutSpec::parse(invalid).unwrap_err().code(),
            ErrorCode::ShortcutInvalid
        );
    }
}

#[test]
fn shortcut_change_registers_new_before_retiring_old() {
    let hotkeys = FakeHotkeys::default();
    let settings = FakeSettings::default();
    let old = ShortcutSpec::parse("Ctrl+Alt+N").unwrap();
    let changed =
        replace_shortcut_atomically(&hotkeys, &settings, Some(&old), "Ctrl+Alt+M", 10).unwrap();

    assert_eq!(changed.as_str(), "Ctrl+Alt+M");
    assert_eq!(
        *hotkeys.calls.lock().unwrap(),
        ["register:Ctrl+Alt+M", "unregister:Ctrl+Alt+N"]
    );
    assert_eq!(
        settings.load_quick_capture_shortcut().unwrap().as_deref(),
        Some("Ctrl+Alt+M")
    );
}

#[test]
fn registration_conflict_leaves_old_shortcut_untouched() {
    let hotkeys = FakeHotkeys::default();
    *hotkeys.fail_register.lock().unwrap() = Some("Ctrl+Alt+M".to_owned());
    let settings = FakeSettings::default();
    let old = ShortcutSpec::parse("Ctrl+Alt+N").unwrap();
    let error =
        replace_shortcut_atomically(&hotkeys, &settings, Some(&old), "Ctrl+Alt+M", 10).unwrap_err();

    assert_eq!(error.code(), ErrorCode::ShortcutConflict);
    assert_eq!(*hotkeys.calls.lock().unwrap(), ["register:Ctrl+Alt+M"]);
}

#[test]
fn old_unregister_failure_rolls_back_new_shortcut() {
    let hotkeys = FakeHotkeys::default();
    *hotkeys.fail_unregister.lock().unwrap() = Some("Ctrl+Alt+N".to_owned());
    let settings = FakeSettings::default();
    let old = ShortcutSpec::parse("Ctrl+Alt+N").unwrap();
    let error =
        replace_shortcut_atomically(&hotkeys, &settings, Some(&old), "Ctrl+Alt+M", 10).unwrap_err();

    assert_eq!(error.code(), ErrorCode::ShortcutConflict);
    assert_eq!(
        *hotkeys.calls.lock().unwrap(),
        [
            "register:Ctrl+Alt+M",
            "unregister:Ctrl+Alt+N",
            "unregister:Ctrl+Alt+M",
        ]
    );
}

#[test]
fn settings_failure_restores_old_and_unregisters_new() {
    let hotkeys = FakeHotkeys::default();
    let settings = FakeSettings::default();
    *settings.fail_save.lock().unwrap() = true;
    let old = ShortcutSpec::parse("Ctrl+Alt+N").unwrap();
    let error =
        replace_shortcut_atomically(&hotkeys, &settings, Some(&old), "Ctrl+Alt+M", 10).unwrap_err();

    assert_eq!(error.code(), ErrorCode::InternalError);
    assert_eq!(
        *hotkeys.calls.lock().unwrap(),
        [
            "register:Ctrl+Alt+M",
            "unregister:Ctrl+Alt+N",
            "register:Ctrl+Alt+N",
            "unregister:Ctrl+Alt+M",
        ]
    );
}

#[test]
fn a_rapid_second_trigger_is_coalesced_without_a_second_clipboard_read() {
    let coordinator = CaptureCoordinator::default();
    let clipboard = CountingClipboard {
        calls: AtomicUsize::new(0),
    };

    assert!(matches!(
        coordinator
            .trigger("session-1".to_owned(), &clipboard)
            .unwrap(),
        CaptureTrigger::Started
    ));
    assert!(matches!(
        coordinator
            .trigger("session-2".to_owned(), &clipboard)
            .unwrap(),
        CaptureTrigger::AlreadyActive
    ));
    assert_eq!(clipboard.calls.load(Ordering::SeqCst), 1);

    let session = coordinator.take_pending().unwrap().unwrap();
    assert_eq!(session.id, "session-1");
    assert_eq!(session.acquisition_attempts, 1);
    assert!(
        matches!(session.clipboard, CapturedClipboard::Text(ref value) if value == "synthetic capture")
    );
    assert!(coordinator.take_pending().unwrap().is_none());
    coordinator.finish("session-1").unwrap();
    assert!(!coordinator.is_active());
}

#[test]
fn only_the_active_capture_session_can_finish() {
    let coordinator = CaptureCoordinator::default();
    let clipboard = CountingClipboard {
        calls: AtomicUsize::new(0),
    };
    coordinator
        .trigger("session-1".to_owned(), &clipboard)
        .unwrap();
    assert_eq!(
        coordinator.finish("session-other").unwrap_err().code(),
        ErrorCode::CaptureSessionMismatch
    );
    assert!(coordinator.is_active());
}
