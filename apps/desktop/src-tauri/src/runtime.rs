use std::{
    path::PathBuf,
    sync::Mutex,
    time::{Duration, Instant, SystemTime},
};

use desktop_notes_core::{
    CalendarService, CaptureCoordinator, CaptureSession, CaptureTrigger, Clock,
    DEFAULT_QUICK_CAPTURE_SHORTCUT, ErrorCode, FoundationError, FoundationService, HotkeyRegistrar,
    ImageAssetService, KeyDeriver, KeyProtector, NoteIdGenerator, NoteService, OrganizationService,
    SearchService, SecretKey, ShortcutSettingsStore, ShortcutSpec, replace_shortcut_atomically,
};
use desktop_notes_infra::{
    AutomaticBackupManager, BackupHealth, BackupRunOutcome, BackupTrigger, EncryptedAssetFiles,
    HkdfKeyDeriver, SafeJsonlLogger, SessionDateUndoStore, SqlCipherStore, SystemClock,
    UuidV4Generator,
};
use desktop_notes_windows::{DpapiKeyring, WindowsClipboard, local_date_today};
use tauri::{App, AppHandle, Emitter, Manager};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut};

use crate::ipc::{IpcState, ShortcutRuntimeStatus};

pub struct AppRuntimeState {
    pub ipc: IpcState,
    database: Option<SqlCipherStore>,
    database_key: Option<SecretKey>,
    asset_files: Option<EncryptedAssetFiles>,
    backup: Option<AutomaticBackupManager>,
    _keyring: Option<DpapiKeyring>,
    _logger: Option<SafeJsonlLogger>,
    ids: UuidV4Generator,
    clock: SystemClock,
    date_undo: SessionDateUndoStore,
    capture: CaptureCoordinator,
    clipboard: WindowsClipboard,
    shortcut: Mutex<ShortcutRuntimeState>,
    last_interval_check: Mutex<Option<Instant>>,
}

struct ShortcutRuntimeState {
    configured: ShortcutSpec,
    active: Option<ShortcutSpec>,
    active_id: Option<u32>,
    registration_error: Option<ErrorCode>,
}

impl Default for ShortcutRuntimeState {
    fn default() -> Self {
        Self {
            configured: default_shortcut(),
            active: None,
            active_id: None,
            registration_error: None,
        }
    }
}

impl AppRuntimeState {
    fn failed(error: FoundationError) -> Self {
        Self {
            ipc: IpcState::failed(error),
            database: None,
            database_key: None,
            asset_files: None,
            backup: None,
            _keyring: None,
            _logger: None,
            ids: UuidV4Generator,
            clock: SystemClock,
            date_undo: SessionDateUndoStore::default(),
            capture: CaptureCoordinator::default(),
            clipboard: WindowsClipboard::default(),
            shortcut: Mutex::new(ShortcutRuntimeState::default()),
            last_interval_check: Mutex::new(None),
        }
    }

    pub fn note_service(&self) -> Result<NoteService<'_>, FoundationError> {
        let database = self.database.as_ref().ok_or_else(|| {
            FoundationError::new(
                ErrorCode::InternalError,
                "The secure Note store is unavailable.",
            )
        })?;
        Ok(NoteService::new(database, &self.ids, &self.clock))
    }

    pub fn load_window_preferences(&self) -> Result<Option<String>, FoundationError> {
        self.database
            .as_ref()
            .ok_or_else(runtime_state_error)?
            .load_window_preferences()
    }

    pub fn save_window_preferences(&self, value_json: &str) -> Result<(), FoundationError> {
        let updated_at_ms = self.clock.now_utc_ms()?;
        self.database
            .as_ref()
            .ok_or_else(runtime_state_error)?
            .save_window_preferences(value_json, updated_at_ms)
    }

    pub fn calendar_service(&self) -> Result<CalendarService<'_>, FoundationError> {
        let database = self.database.as_ref().ok_or_else(|| {
            FoundationError::new(
                ErrorCode::InternalError,
                "The secure Calendar store is unavailable.",
            )
        })?;
        Ok(CalendarService::new(
            database,
            &self.ids,
            &self.clock,
            &self.date_undo,
        ))
    }

    pub fn organization_service(&self) -> Result<OrganizationService<'_>, FoundationError> {
        let database = self.database.as_ref().ok_or_else(|| {
            FoundationError::new(
                ErrorCode::InternalError,
                "The secure organization store is unavailable.",
            )
        })?;
        Ok(OrganizationService::new(database, &self.ids, &self.clock))
    }

    pub fn search_service(&self) -> Result<SearchService<'_>, FoundationError> {
        let database = self.database.as_ref().ok_or_else(|| {
            FoundationError::new(
                ErrorCode::InternalError,
                "The secure Search store is unavailable.",
            )
        })?;
        Ok(SearchService::new(database))
    }

    pub fn image_asset_service(&self) -> Result<ImageAssetService<'_>, FoundationError> {
        let database = self.database.as_ref().ok_or_else(|| {
            FoundationError::new(
                ErrorCode::InternalError,
                "The secure image metadata store is unavailable.",
            )
        })?;
        let files = self.asset_files.as_ref().ok_or_else(|| {
            FoundationError::new(
                ErrorCode::InternalError,
                "The encrypted image store is unavailable.",
            )
        })?;
        Ok(ImageAssetService::new(
            database,
            files,
            &self.ids,
            &self.clock,
        ))
    }

    pub fn initialize_shortcut(&self, app: &AppHandle) {
        let Some(database) = self.database.as_ref() else {
            return;
        };
        let (configured, mut registration_error) = match database.load_quick_capture_shortcut() {
            Ok(Some(value)) => match ShortcutSpec::parse(&value) {
                Ok(shortcut) => (shortcut, None),
                Err(error) => (default_shortcut(), Some(error.code())),
            },
            Ok(None) => (default_shortcut(), None),
            Err(error) => (default_shortcut(), Some(error.code())),
        };
        let registrar = TauriHotkeyRegistrar { app };
        let active = match registrar.register(&configured) {
            Ok(()) => Some(configured.clone()),
            Err(error) => {
                registration_error = Some(error.code());
                let fallback = default_shortcut();
                if fallback != configured && registrar.register(&fallback).is_ok() {
                    Some(fallback)
                } else {
                    None
                }
            }
        };

        if let Ok(mut state) = self.shortcut.lock() {
            state.configured = configured;
            state.active_id = active.as_ref().and_then(shortcut_id);
            state.active = active;
            state.registration_error = registration_error;
        }
    }

    pub fn configure_shortcut(
        &self,
        app: &AppHandle,
        requested: &str,
    ) -> Result<ShortcutRuntimeStatus, FoundationError> {
        let database = self.database.as_ref().ok_or_else(|| {
            FoundationError::new(
                ErrorCode::InternalError,
                "The secure shortcut settings store is unavailable.",
            )
        })?;
        let updated_at_ms = self.clock.now_utc_ms()?;
        let registrar = TauriHotkeyRegistrar { app };
        let mut state = self.shortcut.lock().map_err(|_| runtime_state_error())?;
        let result = replace_shortcut_atomically(
            &registrar,
            database,
            state.active.as_ref(),
            requested,
            updated_at_ms,
        );
        match result {
            Ok(next) => {
                state.configured = next.clone();
                state.active_id = shortcut_id(&next);
                state.active = Some(next);
                state.registration_error = None;
                Ok(shortcut_status(&state))
            }
            Err(error) => {
                state.registration_error = Some(error.code());
                Err(error)
            }
        }
    }

    pub fn shortcut_status(&self) -> Result<ShortcutRuntimeStatus, FoundationError> {
        self.shortcut
            .lock()
            .map(|state| shortcut_status(&state))
            .map_err(|_| runtime_state_error())
    }

    pub fn is_active_shortcut(&self, shortcut_id: u32) -> bool {
        self.shortcut
            .lock()
            .map(|state| state.active_id == Some(shortcut_id))
            .unwrap_or(false)
    }

    pub fn trigger_quick_capture(&self) -> Result<CaptureTrigger, FoundationError> {
        let session_id = self.ids.new_note_id()?;
        self.capture.trigger(session_id, &self.clipboard)
    }

    pub fn take_quick_capture(&self) -> Result<Option<CaptureSession>, FoundationError> {
        self.capture.take_pending()
    }

    pub fn finish_quick_capture(&self, session_id: &str) -> Result<(), FoundationError> {
        self.capture.finish(session_id)
    }

    pub fn backup_health(&self) -> Result<BackupHealth, FoundationError> {
        self.backup
            .as_ref()
            .map(AutomaticBackupManager::health)
            .ok_or_else(FoundationError::backup_failed)
    }

    pub fn run_automatic_backup(
        &self,
        trigger: BackupTrigger,
    ) -> Result<BackupRunOutcome, FoundationError> {
        let database = self
            .database
            .as_ref()
            .ok_or_else(FoundationError::backup_failed)?;
        let database_key = self
            .database_key
            .as_ref()
            .ok_or_else(FoundationError::backup_failed)?;
        let assets = self
            .asset_files
            .as_ref()
            .ok_or_else(FoundationError::backup_failed)?;
        let backup = self
            .backup
            .as_ref()
            .ok_or_else(FoundationError::backup_failed)?;
        let now_ms = self.clock.now_utc_ms()?;
        let result = backup.run_if_due(
            database,
            database_key,
            assets,
            now_ms,
            &local_date_today()?,
            trigger,
        );
        let _ = backup.cleanup_invalid(now_ms);
        if result.is_ok() {
            let _ = backup.refresh_health();
        }
        result
    }

    fn claim_interval_check(&self) -> bool {
        let Ok(mut last) = self.last_interval_check.lock() else {
            return false;
        };
        let now = Instant::now();
        if last.is_some_and(|previous| now.duration_since(previous) < Duration::from_secs(15 * 60))
        {
            return false;
        }
        *last = Some(now);
        true
    }
}

pub fn spawn_automatic_backup(app: &AppHandle, trigger: BackupTrigger) {
    let Some(state) = app.try_state::<AppRuntimeState>() else {
        return;
    };
    if trigger == BackupTrigger::Interval && !state.claim_interval_check() {
        return;
    }
    let app = app.clone();
    std::thread::spawn(move || {
        let Some(state) = app.try_state::<AppRuntimeState>() else {
            return;
        };
        let _ = state.run_automatic_backup(trigger);
        if let Ok(status) = state.backup_health() {
            let _ = app.emit("desktop-notes://backup-status", status);
        }
    });
}

pub fn bootstrap(app: &App) -> AppRuntimeState {
    let root = match app.path().app_local_data_dir() {
        Ok(root) => root,
        Err(_) => {
            return AppRuntimeState::failed(FoundationError::new(
                ErrorCode::DataRootUnavailable,
                "The local application data directory is unavailable.",
            ));
        }
    };
    bootstrap_at(root)
}

fn bootstrap_at(root: PathBuf) -> AppRuntimeState {
    let logger = match SafeJsonlLogger::new(root.join("logs").join("events.jsonl")) {
        Ok(logger) => logger,
        Err(error) => return AppRuntimeState::failed(error),
    };
    let keyring = DpapiKeyring::new(root.join("control").join("keyring.json"));
    let database = SqlCipherStore::new(
        root.join("data").join("desktop-notes.db"),
        env!("CARGO_PKG_VERSION"),
    );
    let deriver = HkdfKeyDeriver;
    let operation_id = startup_operation_id();
    let result =
        FoundationService::new(&keyring, &deriver, &database, &logger).initialize(&operation_id);
    let (ipc, database, database_key, asset_files, backup) = match result {
        Ok(status) => {
            let asset_setup = (|| {
                let root_key = keyring
                    .load_key()?
                    .ok_or_else(FoundationError::key_unavailable)?;
                let database_key = deriver.derive_database_key(&root_key)?;
                let asset_key = deriver.derive_asset_key(&root_key)?;
                let files = EncryptedAssetFiles::new(root.clone(), asset_key);
                ImageAssetService::new(&database, &files, &UuidV4Generator, &SystemClock)
                    .repair_at_startup()?;
                let backup = (|| {
                    let backup_key = deriver.derive_backup_key(&root_key)?;
                    let wrapped_root = keyring.export_wrapped_root()?;
                    let manager =
                        AutomaticBackupManager::new(root.clone(), backup_key, wrapped_root)?;
                    let _ = manager.refresh_health();
                    Ok::<_, FoundationError>(manager)
                })()
                .ok();
                Ok::<_, FoundationError>((files, database_key, backup))
            })();
            match asset_setup {
                Ok((files, database_key, backup)) => {
                    let ipc = if let Some(manager) = backup.as_ref() {
                        IpcState::ready_with_backup(status, manager.health())
                    } else {
                        IpcState::ready(status)
                    };
                    (ipc, Some(database), Some(database_key), Some(files), backup)
                }
                Err(error) => (IpcState::failed(error), None, None, None, None),
            }
        }
        Err(error) => (IpcState::failed(error), None, None, None, None),
    };

    AppRuntimeState {
        ipc,
        database,
        database_key,
        asset_files,
        backup,
        _keyring: Some(keyring),
        _logger: Some(logger),
        ids: UuidV4Generator,
        clock: SystemClock,
        date_undo: SessionDateUndoStore::default(),
        capture: CaptureCoordinator::default(),
        clipboard: WindowsClipboard::default(),
        shortcut: Mutex::new(ShortcutRuntimeState::default()),
        last_interval_check: Mutex::new(None),
    }
}

struct TauriHotkeyRegistrar<'a> {
    app: &'a AppHandle,
}

impl HotkeyRegistrar for TauriHotkeyRegistrar<'_> {
    fn register(&self, shortcut: &ShortcutSpec) -> Result<(), FoundationError> {
        self.app
            .global_shortcut()
            .register(shortcut.as_str())
            .map_err(|_| FoundationError::shortcut_conflict())
    }

    fn unregister(&self, shortcut: &ShortcutSpec) -> Result<(), FoundationError> {
        self.app
            .global_shortcut()
            .unregister(shortcut.as_str())
            .map_err(|_| FoundationError::shortcut_conflict())
    }
}

fn shortcut_status(state: &ShortcutRuntimeState) -> ShortcutRuntimeStatus {
    ShortcutRuntimeStatus {
        configured_shortcut: state.configured.as_str().to_owned(),
        active_shortcut: state
            .active
            .as_ref()
            .map(|shortcut| shortcut.as_str().to_owned()),
        registration_error: state.registration_error,
    }
}

fn shortcut_id(shortcut: &ShortcutSpec) -> Option<u32> {
    shortcut
        .as_str()
        .parse::<Shortcut>()
        .ok()
        .map(|shortcut| shortcut.id())
}

fn default_shortcut() -> ShortcutSpec {
    ShortcutSpec::parse(DEFAULT_QUICK_CAPTURE_SHORTCUT)
        .expect("the frozen default shortcut must remain valid")
}

fn runtime_state_error() -> FoundationError {
    FoundationError::new(
        ErrorCode::InternalError,
        "The desktop quick-capture runtime is unavailable.",
    )
}

fn startup_operation_id() -> String {
    let millis = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or_default();
    format!("startup-{}-{millis}", std::process::id())
}
