use std::sync::Mutex;

use desktop_notes_core::{ErrorCode, FoundationError};
use desktop_notes_windows::{
    DockEdge, MaterialKind, MaterialStatus, MonitorLayout, PersistedPlacement, Rect, VisualState,
    apply_window_material, detect_edge_snap, recover_placement,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tauri::{
    AppHandle, LogicalSize, Manager, PhysicalPosition, PhysicalSize, State, WebviewWindow,
};

use crate::{ipc::IpcErrorDto, runtime::AppRuntimeState};

const PROTOCOL_VERSION: u16 = 1;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThemeMode {
    #[default]
    System,
    Light,
    Dark,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct WindowPreferences {
    version: u16,
    visual_state: VisualState,
    theme: ThemeMode,
    collapsed: Option<PersistedPlacement>,
    normal: Option<PersistedPlacement>,
    expanded: Option<PersistedPlacement>,
}

impl Default for WindowPreferences {
    fn default() -> Self {
        Self {
            version: 1,
            visual_state: VisualState::Normal,
            theme: ThemeMode::System,
            collapsed: None,
            normal: None,
            expanded: None,
        }
    }
}

impl WindowPreferences {
    fn placement(&self, state: VisualState) -> Option<&PersistedPlacement> {
        match state {
            VisualState::Collapsed => self.collapsed.as_ref(),
            VisualState::Normal => self.normal.as_ref(),
            VisualState::Expanded => self.expanded.as_ref(),
        }
    }

    fn set_placement(&mut self, placement: PersistedPlacement) {
        match placement.visual_state {
            VisualState::Collapsed => self.collapsed = Some(placement),
            VisualState::Normal => self.normal = Some(placement),
            VisualState::Expanded => self.expanded = Some(placement),
        }
    }
}

pub struct WindowRuntimeState {
    preferences: Mutex<WindowPreferences>,
    material: Mutex<MaterialStatus>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WindowSessionDto {
    protocol_version: u16,
    visual_state: VisualState,
    theme: ThemeMode,
    material: MaterialKind,
    material_reason: String,
    topmost: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WindowEnvelope {
    ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    session: Option<WindowSessionDto>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<IpcErrorDto>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct VersionRequest {
    protocol_version: u16,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct StateRequest {
    protocol_version: u16,
    visual_state: VisualState,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct ThemeRequest {
    protocol_version: u16,
    theme: ThemeMode,
}

pub fn initialize(app: &AppHandle) {
    let preferences = app
        .state::<AppRuntimeState>()
        .load_window_preferences()
        .ok()
        .flatten()
        .and_then(|raw| serde_json::from_str::<WindowPreferences>(&raw).ok())
        .filter(|value| value.version == 1)
        .unwrap_or_default();
    let fallback_material = MaterialStatus {
        selected: MaterialKind::Solid,
        reason: "window_not_ready".to_owned(),
        api_roundtrip_verified: false,
    };
    app.manage(WindowRuntimeState {
        preferences: Mutex::new(preferences),
        material: Mutex::new(fallback_material),
    });

    let Some(window) = app.get_webview_window("main") else {
        return;
    };
    // Topmost is deliberately session-only. Every real process start resets it here.
    let _ = window.set_always_on_top(false);
    let _ = restore_current_state(&window, &app.state::<WindowRuntimeState>());
    if let Ok(hwnd) = window.hwnd() {
        let status = apply_window_material(hwnd.0 as isize);
        if let Ok(mut material) = app.state::<WindowRuntimeState>().material.lock() {
            *material = status;
        }
    }
}

pub fn get_window_session(request: Value, state: State<'_, WindowRuntimeState>) -> WindowEnvelope {
    match parse::<VersionRequest>(request).map(|request| request.protocol_version) {
        Ok(PROTOCOL_VERSION) => session_response(&state),
        _ => invalid_request(),
    }
}

pub fn set_visual_state(
    request: Value,
    window: WebviewWindow,
    app_state: State<'_, AppRuntimeState>,
    state: State<'_, WindowRuntimeState>,
) -> WindowEnvelope {
    let request = match parse::<StateRequest>(request) {
        Ok(request) if request.protocol_version == PROTOCOL_VERSION => request,
        _ => return invalid_request(),
    };
    if let Err(error) = save_current_placement(&window, &app_state, &state) {
        return error_response(error);
    }
    if let Ok(mut preferences) = state.preferences.lock() {
        preferences.visual_state = request.visual_state;
    } else {
        return runtime_error();
    }
    if let Err(error) = restore_current_state(&window, &state) {
        return error_response(error);
    }
    if let Err(error) = persist(&app_state, &state) {
        return error_response(error);
    }
    session_response(&state)
}

pub fn set_theme_mode(
    request: Value,
    app_state: State<'_, AppRuntimeState>,
    state: State<'_, WindowRuntimeState>,
) -> WindowEnvelope {
    let request = match parse::<ThemeRequest>(request) {
        Ok(request) if request.protocol_version == PROTOCOL_VERSION => request,
        _ => return invalid_request(),
    };
    if let Ok(mut preferences) = state.preferences.lock() {
        preferences.theme = request.theme;
    } else {
        return runtime_error();
    }
    match persist(&app_state, &state) {
        Ok(()) => session_response(&state),
        Err(error) => error_response(error),
    }
}

pub fn save_window_placement(
    request: Value,
    window: WebviewWindow,
    app_state: State<'_, AppRuntimeState>,
    state: State<'_, WindowRuntimeState>,
) -> WindowEnvelope {
    match parse::<VersionRequest>(request).map(|request| request.protocol_version) {
        Ok(PROTOCOL_VERSION) => match save_current_placement(&window, &app_state, &state) {
            Ok(()) => session_response(&state),
            Err(error) => error_response(error),
        },
        _ => invalid_request(),
    }
}

fn save_current_placement(
    window: &WebviewWindow,
    app_state: &AppRuntimeState,
    state: &WindowRuntimeState,
) -> Result<(), FoundationError> {
    let position = window.outer_position().map_err(|_| window_error())?;
    let size = window.outer_size().map_err(|_| window_error())?;
    let monitor = window
        .current_monitor()
        .map_err(|_| window_error())?
        .or_else(|| window.primary_monitor().ok().flatten())
        .ok_or_else(window_error)?;
    let primary = window.primary_monitor().ok().flatten();
    let monitor_layout = monitor_layout(&monitor, primary.as_ref());
    let rect = Rect {
        x: i64::from(position.x),
        y: i64::from(position.y),
        width: i64::from(size.width),
        height: i64::from(size.height),
    };
    let threshold = (12.0 * monitor.scale_factor()).round() as i64;
    let dock_edge = detect_edge_snap(rect, monitor_layout.work_area, threshold);
    let vertical_offset_milli = dock_edge.and_then(|edge| match edge {
        DockEdge::Left | DockEdge::Right => {
            let room = monitor_layout.work_area.height.saturating_sub(rect.height);
            (room > 0).then(|| {
                (((rect.y - monitor_layout.work_area.y).clamp(0, room) * 1_000) / room) as u16
            })
        }
        DockEdge::Top | DockEdge::Bottom => None,
    });
    let visual_state = state
        .preferences
        .lock()
        .map_err(|_| window_error())?
        .visual_state;
    let placement = PersistedPlacement {
        monitor_fingerprint: Some(monitor_layout.fingerprint),
        monitor_fallback_name: Some(monitor_layout.fallback_name),
        rect,
        saved_scale_milli: monitor_layout.scale_milli,
        visual_state,
        dock_edge,
        vertical_offset_milli,
    };
    state
        .preferences
        .lock()
        .map_err(|_| window_error())?
        .set_placement(placement);
    persist(app_state, state)
}

fn restore_current_state(
    window: &WebviewWindow,
    state: &WindowRuntimeState,
) -> Result<(), FoundationError> {
    let primary = window.primary_monitor().map_err(|_| window_error())?;
    let monitors = window
        .available_monitors()
        .map_err(|_| window_error())?
        .iter()
        .map(|monitor| monitor_layout(monitor, primary.as_ref()))
        .collect::<Vec<_>>();
    let preferences = state.preferences.lock().map_err(|_| window_error())?;
    let visual_state = preferences.visual_state;
    let resolved = recover_placement(preferences.placement(visual_state), &monitors, visual_state)
        .map_err(|_| window_error())?;
    drop(preferences);

    let (minimum_width, minimum_height, resizable) = match visual_state {
        VisualState::Collapsed => (288, 112, false),
        VisualState::Normal => (640, 520, true),
        VisualState::Expanded => (900, 620, true),
    };
    window
        .set_min_size(Some(LogicalSize::new(minimum_width, minimum_height)))
        .map_err(|_| window_error())?;
    window
        .set_resizable(resizable)
        .map_err(|_| window_error())?;
    let current_outer = window.outer_size().map_err(|_| window_error())?;
    let current_inner = window.inner_size().map_err(|_| window_error())?;
    let target_inner = outer_to_inner_size(
        PhysicalSize::new(resolved.rect.width as u32, resolved.rect.height as u32),
        current_outer,
        current_inner,
    );
    window.set_size(target_inner).map_err(|_| window_error())?;
    window
        .set_position(PhysicalPosition::new(
            resolved.rect.x as i32,
            resolved.rect.y as i32,
        ))
        .map_err(|_| window_error())?;
    Ok(())
}

fn outer_to_inner_size(
    target_outer: PhysicalSize<u32>,
    current_outer: PhysicalSize<u32>,
    current_inner: PhysicalSize<u32>,
) -> PhysicalSize<u32> {
    let chrome_width = current_outer.width.saturating_sub(current_inner.width);
    let chrome_height = current_outer.height.saturating_sub(current_inner.height);
    PhysicalSize::new(
        target_outer.width.saturating_sub(chrome_width).max(1),
        target_outer.height.saturating_sub(chrome_height).max(1),
    )
}

fn monitor_layout(monitor: &tauri::Monitor, primary: Option<&tauri::Monitor>) -> MonitorLayout {
    let work = monitor.work_area();
    let fallback_name = monitor
        .name()
        .cloned()
        .unwrap_or_else(|| "unnamed-monitor".to_owned());
    let fingerprint = format!(
        "{}:{}:{}:{}:{}",
        fallback_name,
        monitor.position().x,
        monitor.position().y,
        monitor.size().width,
        monitor.size().height
    );
    let is_primary = primary.is_some_and(|value| {
        value.position() == monitor.position() && value.size() == monitor.size()
    });
    MonitorLayout {
        fingerprint,
        fallback_name,
        work_area: Rect {
            x: i64::from(work.position.x),
            y: i64::from(work.position.y),
            width: i64::from(work.size.width),
            height: i64::from(work.size.height),
        },
        scale_milli: (monitor.scale_factor() * 1_000.0)
            .round()
            .clamp(500.0, 4_000.0) as u32,
        primary: is_primary,
    }
}

fn persist(app_state: &AppRuntimeState, state: &WindowRuntimeState) -> Result<(), FoundationError> {
    let preferences = state.preferences.lock().map_err(|_| window_error())?;
    let json = serde_json::to_string(&*preferences).map_err(|_| window_error())?;
    drop(preferences);
    app_state.save_window_preferences(&json)
}

fn session_response(state: &WindowRuntimeState) -> WindowEnvelope {
    let preferences = match state.preferences.lock() {
        Ok(value) => value,
        Err(_) => return runtime_error(),
    };
    let material = match state.material.lock() {
        Ok(value) => value,
        Err(_) => return runtime_error(),
    };
    WindowEnvelope {
        ok: true,
        session: Some(WindowSessionDto {
            protocol_version: PROTOCOL_VERSION,
            visual_state: preferences.visual_state,
            theme: preferences.theme,
            material: material.selected,
            material_reason: material.reason.clone(),
            topmost: false,
        }),
        error: None,
    }
}

fn parse<T: for<'de> Deserialize<'de>>(request: Value) -> Result<T, serde_json::Error> {
    serde_json::from_value(request)
}

fn invalid_request() -> WindowEnvelope {
    error_response(FoundationError::validation_failed())
}

fn runtime_error() -> WindowEnvelope {
    error_response(window_error())
}

fn error_response(error: FoundationError) -> WindowEnvelope {
    WindowEnvelope {
        ok: false,
        session: None,
        error: Some(IpcErrorDto {
            code: error.code(),
            message: error.safe_message().to_owned(),
            recoverable: false,
        }),
    }
}

fn window_error() -> FoundationError {
    FoundationError::new(
        ErrorCode::InternalError,
        "The desktop window could not be updated safely.",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn restoring_an_outer_size_does_not_accumulate_window_chrome() {
        assert_eq!(
            outer_to_inner_size(
                PhysicalSize::new(1_280, 820),
                PhysicalSize::new(918, 767),
                PhysicalSize::new(900, 720),
            ),
            PhysicalSize::new(1_262, 773)
        );
    }

    #[test]
    fn preferences_are_versioned_and_reject_unknown_fields() {
        let value = serde_json::to_value(WindowPreferences::default()).unwrap();
        assert_eq!(value["version"], 1);
        assert_eq!(value["visualState"], "normal");
        assert_eq!(value["theme"], "system");
        assert!(
            parse::<ThemeRequest>(serde_json::json!({
                "protocolVersion": 1,
                "theme": "dark",
                "extra": true
            }))
            .is_err()
        );
    }

    #[test]
    fn each_visual_state_has_an_independent_placement_slot() {
        let mut preferences = WindowPreferences::default();
        for (state, x) in [
            (VisualState::Collapsed, 10),
            (VisualState::Normal, 20),
            (VisualState::Expanded, 30),
        ] {
            preferences.set_placement(PersistedPlacement {
                monitor_fingerprint: Some("display".to_owned()),
                monitor_fallback_name: Some("DISPLAY1".to_owned()),
                rect: Rect {
                    x,
                    y: 0,
                    width: 500,
                    height: 400,
                },
                saved_scale_milli: 1_000,
                visual_state: state,
                dock_edge: None,
                vertical_offset_milli: None,
            });
        }
        assert_eq!(
            preferences
                .placement(VisualState::Collapsed)
                .unwrap()
                .rect
                .x,
            10
        );
        assert_eq!(
            preferences.placement(VisualState::Normal).unwrap().rect.x,
            20
        );
        assert_eq!(
            preferences.placement(VisualState::Expanded).unwrap().rect.x,
            30
        );
    }
}
