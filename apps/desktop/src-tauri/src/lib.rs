pub mod ipc;
mod runtime;
pub mod windowing;

use desktop_notes_core::{ErrorCode, FoundationError};
use serde_json::Value;
use tauri::{AppHandle, Emitter, Manager, RunEvent, State};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};

use desktop_notes_infra::BackupTrigger;
use ipc::{
    ActionEnvelope, BackupEnvelope, CaptureEnvelope, FoundationEnvelope, NoteEnvelope,
    ShortcutEnvelope,
};
use runtime::AppRuntimeState;
use windowing::{WindowEnvelope, WindowRuntimeState};

#[tauri::command]
fn get_foundation_status(request: Value, state: State<'_, AppRuntimeState>) -> FoundationEnvelope {
    ipc::handle_get_foundation_status(request, &state.ipc)
}

#[tauri::command]
fn create_note(request: Value, state: State<'_, AppRuntimeState>) -> NoteEnvelope {
    match state.note_service() {
        Ok(service) => ipc::handle_create_note(request, &service),
        Err(error) => ipc::note_error(error),
    }
}

#[tauri::command]
fn get_note(request: Value, state: State<'_, AppRuntimeState>) -> NoteEnvelope {
    match state.note_service() {
        Ok(service) => ipc::handle_get_note(request, &service),
        Err(error) => ipc::note_error(error),
    }
}

#[tauri::command]
fn delete_note(request: Value, state: State<'_, AppRuntimeState>) -> NoteEnvelope {
    match state.note_service() {
        Ok(service) => ipc::handle_delete_note(request, &service),
        Err(error) => ipc::note_error(error),
    }
}

#[tauri::command]
fn list_notes_for_date(request: Value, state: State<'_, AppRuntimeState>) -> NoteEnvelope {
    match state.note_service() {
        Ok(service) => ipc::handle_list_notes_for_date(request, &service),
        Err(error) => ipc::note_error(error),
    }
}

#[tauri::command]
fn update_note_content(request: Value, state: State<'_, AppRuntimeState>) -> NoteEnvelope {
    match state.note_service() {
        Ok(service) => ipc::handle_update_note_content(request, &service),
        Err(error) => ipc::note_error(error),
    }
}

#[tauri::command]
fn list_note_counts_for_month(request: Value, state: State<'_, AppRuntimeState>) -> NoteEnvelope {
    match state.calendar_service() {
        Ok(service) => ipc::handle_list_note_counts_for_month(request, &service),
        Err(error) => ipc::note_error(error),
    }
}

#[tauri::command]
fn change_note_date(request: Value, state: State<'_, AppRuntimeState>) -> NoteEnvelope {
    match state.calendar_service() {
        Ok(service) => ipc::handle_change_note_date(request, &service),
        Err(error) => ipc::note_error(error),
    }
}

#[tauri::command]
fn undo_note_date_change(request: Value, state: State<'_, AppRuntimeState>) -> NoteEnvelope {
    match state.calendar_service() {
        Ok(service) => ipc::handle_undo_note_date_change(request, &service),
        Err(error) => ipc::note_error(error),
    }
}

macro_rules! organization_command {
    ($command:ident, $handler:ident) => {
        #[tauri::command]
        fn $command(request: Value, state: State<'_, AppRuntimeState>) -> NoteEnvelope {
            match state.organization_service() {
                Ok(service) => ipc::$handler(request, &service),
                Err(error) => ipc::note_error(error),
            }
        }
    };
}

organization_command!(list_tags, handle_list_tags);
organization_command!(create_tag, handle_create_tag);
organization_command!(rename_tag, handle_rename_tag);
organization_command!(delete_tag, handle_delete_tag);
organization_command!(list_tags_for_note, handle_list_tags_for_note);
organization_command!(assign_tag, handle_assign_tag);
organization_command!(remove_tag, handle_remove_tag);
organization_command!(set_note_pinned, handle_set_note_pinned);
organization_command!(list_recent_notes, handle_list_recent_notes);

#[tauri::command]
fn search_notes(request: Value, state: State<'_, AppRuntimeState>) -> NoteEnvelope {
    match state.search_service() {
        Ok(service) => ipc::handle_search_notes(request, &service),
        Err(error) => ipc::note_error(error),
    }
}

#[tauri::command(async)]
fn import_image_asset(request: Value, state: State<'_, AppRuntimeState>) -> NoteEnvelope {
    match state.image_asset_service() {
        Ok(service) => ipc::handle_import_image_asset(request, &service),
        Err(error) => ipc::note_error(error),
    }
}

#[tauri::command(async)]
fn read_image_asset(request: Value, state: State<'_, AppRuntimeState>) -> NoteEnvelope {
    match state.image_asset_service() {
        Ok(service) => ipc::handle_read_image_asset(request, &service),
        Err(error) => ipc::note_error(error),
    }
}

#[tauri::command(async)]
fn discard_image_asset(request: Value, state: State<'_, AppRuntimeState>) -> ActionEnvelope {
    match state.image_asset_service() {
        Ok(service) => ipc::handle_discard_image_asset(request, &service),
        Err(error) => ipc::action_error(error),
    }
}

#[tauri::command]
fn take_quick_capture(request: Value, state: State<'_, AppRuntimeState>) -> CaptureEnvelope {
    ipc::handle_take_quick_capture(request, || state.take_quick_capture())
}

#[tauri::command]
fn finish_quick_capture(request: Value, state: State<'_, AppRuntimeState>) -> ActionEnvelope {
    ipc::handle_finish_quick_capture(request, |session_id| state.finish_quick_capture(session_id))
}

#[tauri::command]
fn get_shortcut_config(request: Value, state: State<'_, AppRuntimeState>) -> ShortcutEnvelope {
    ipc::handle_get_shortcut_config(request, || state.shortcut_status())
}

#[tauri::command]
fn set_shortcut_config(
    request: Value,
    app: AppHandle,
    state: State<'_, AppRuntimeState>,
) -> ShortcutEnvelope {
    ipc::handle_set_shortcut_config(request, |shortcut| state.configure_shortcut(&app, shortcut))
}

#[tauri::command]
fn get_backup_status(request: Value, state: State<'_, AppRuntimeState>) -> BackupEnvelope {
    ipc::handle_get_backup_status(request, || state.backup_health())
}

#[tauri::command]
fn get_window_session(request: Value, state: State<'_, WindowRuntimeState>) -> WindowEnvelope {
    windowing::get_window_session(request, state)
}

#[tauri::command]
fn set_visual_state(
    request: Value,
    window: tauri::WebviewWindow,
    app_state: State<'_, AppRuntimeState>,
    state: State<'_, WindowRuntimeState>,
) -> WindowEnvelope {
    windowing::set_visual_state(request, window, app_state, state)
}

#[tauri::command]
fn set_theme_mode(
    request: Value,
    app_state: State<'_, AppRuntimeState>,
    state: State<'_, WindowRuntimeState>,
) -> WindowEnvelope {
    windowing::set_theme_mode(request, app_state, state)
}

#[tauri::command]
fn save_window_placement(
    request: Value,
    window: tauri::WebviewWindow,
    app_state: State<'_, AppRuntimeState>,
    state: State<'_, WindowRuntimeState>,
) -> WindowEnvelope {
    windowing::save_window_placement(request, window, app_state, state)
}

pub fn run() -> Result<(), FoundationError> {
    ensure_webview_runtime()?;
    let context = tauri::generate_context!();
    let app_local_data_root =
        desktop_notes_windows::app_local_data_root(&context.config().identifier)?;
    desktop_notes_windows::cleanup_legacy_webview_autofill(&app_local_data_root)?;
    let app = tauri::Builder::default()
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, shortcut, event| {
                    if event.state != ShortcutState::Pressed {
                        return;
                    }
                    let Some(state) = app.try_state::<AppRuntimeState>() else {
                        return;
                    };
                    if !state.is_active_shortcut(shortcut.id()) {
                        return;
                    }
                    if state.trigger_quick_capture().is_err() {
                        return;
                    }
                    if let Some(window) = app.get_webview_window("main") {
                        let _ = window.show();
                        let _ = window.unminimize();
                        let _ = window.set_focus();
                    }
                    let _ = app.emit("desktop-notes://quick-capture-available", ());
                })
                .build(),
        )
        .setup(|app| {
            app.manage(runtime::bootstrap(app));
            windowing::initialize(app.handle());
            app.state::<AppRuntimeState>()
                .initialize_shortcut(app.handle());
            runtime::spawn_automatic_backup(app.handle(), BackupTrigger::Startup);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_foundation_status,
            create_note,
            get_note,
            delete_note,
            list_notes_for_date,
            update_note_content,
            list_note_counts_for_month,
            change_note_date,
            undo_note_date_change,
            list_tags,
            create_tag,
            rename_tag,
            delete_tag,
            list_tags_for_note,
            assign_tag,
            remove_tag,
            set_note_pinned,
            list_recent_notes,
            search_notes,
            import_image_asset,
            read_image_asset,
            discard_image_asset,
            take_quick_capture,
            finish_quick_capture,
            get_shortcut_config,
            set_shortcut_config,
            get_backup_status,
            get_window_session,
            set_visual_state,
            set_theme_mode,
            save_window_placement
        ])
        .build(context)
        .map_err(|error| classify_runtime_error(&error))?;
    app.run(|app, event| match event {
        RunEvent::Resumed => {
            runtime::spawn_automatic_backup(app, BackupTrigger::Resumed);
        }
        RunEvent::MainEventsCleared => {
            runtime::spawn_automatic_backup(app, BackupTrigger::Interval);
        }
        RunEvent::Exit => {
            let _ = app.global_shortcut().unregister_all();
        }
        _ => {}
    });
    Ok(())
}

fn ensure_webview_runtime() -> Result<(), FoundationError> {
    require_webview_version(tauri::webview_version())
}

fn require_webview_version<T, E>(result: Result<T, E>) -> Result<(), FoundationError> {
    result.map(|_| ()).map_err(|_| {
        FoundationError::new(
            ErrorCode::WebviewUnavailable,
            "Microsoft Edge WebView2 Runtime is unavailable.",
        )
    })
}

fn classify_runtime_error(error: &tauri::Error) -> FoundationError {
    match error {
        tauri::Error::Runtime(
            tauri_runtime::Error::WebviewRuntimeNotInstalled
            | tauri_runtime::Error::CreateWebview(_),
        ) => FoundationError::new(
            ErrorCode::WebviewUnavailable,
            "Microsoft Edge WebView2 Runtime is unavailable.",
        ),
        _ => FoundationError::new(
            ErrorCode::InternalError,
            "Desktop Notes could not start its desktop runtime.",
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_webview_runtime_is_classified_without_raw_details() {
        let error = classify_runtime_error(&tauri::Error::Runtime(
            tauri_runtime::Error::WebviewRuntimeNotInstalled,
        ));

        assert_eq!(error.code(), ErrorCode::WebviewUnavailable);
        assert_eq!(
            error.safe_message(),
            "Microsoft Edge WebView2 Runtime is unavailable."
        );
    }

    #[test]
    fn preflight_failure_stops_before_tauri_creates_a_window() {
        let error = require_webview_version::<(), _>(Err("synthetic detail")).unwrap_err();

        assert_eq!(error.code(), ErrorCode::WebviewUnavailable);
        assert_eq!(
            error.safe_message(),
            "Microsoft Edge WebView2 Runtime is unavailable."
        );
        assert!(!error.safe_message().contains("synthetic"));
    }

    #[test]
    fn unrelated_runtime_failure_is_not_misclassified_as_webview() {
        let error = classify_runtime_error(&tauri::Error::WebviewNotFound);

        assert_eq!(error.code(), ErrorCode::InternalError);
        assert_eq!(
            error.safe_message(),
            "Desktop Notes could not start its desktop runtime."
        );
    }
}
