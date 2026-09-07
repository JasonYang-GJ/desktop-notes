//! Windows-only platform adapters.

mod clipboard;
mod clock;
mod keyring;
mod placement;
mod startup_error;
mod visual_effects;
mod webview_data;

pub use clipboard::WindowsClipboard;
pub use clock::local_date_today;
pub use keyring::DpapiKeyring;
pub use placement::{
    DockEdge, MonitorLayout, PersistedPlacement, PlacementError, Rect, ResolvedPlacement,
    VisualState, detect_edge_snap, recover_placement,
};
pub use startup_error::{
    StartupFallback, show_startup_failure, show_webview_unavailable, webview_unavailable_fallback,
};
pub use visual_effects::{MaterialKind, MaterialStatus, apply_window_material};
pub use webview_data::{app_local_data_root, cleanup_legacy_webview_autofill};
