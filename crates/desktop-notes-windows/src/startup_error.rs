use desktop_notes_core::ErrorCode;

const TITLE: &str = "桌面笔记";
const MESSAGE: &str = "桌面笔记需要 Microsoft Edge WebView2 Runtime。请安装或修复 WebView2，然后重启应用。\n\n错误代码：WEBVIEW_UNAVAILABLE";
const INTERNAL_MESSAGE: &str =
    "桌面笔记无法启动桌面运行环境。请重启应用。\n\n错误代码：INTERNAL_ERROR";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StartupFallback {
    pub code: ErrorCode,
    pub title: &'static str,
    pub message: &'static str,
}

pub const fn webview_unavailable_fallback() -> StartupFallback {
    StartupFallback {
        code: ErrorCode::WebviewUnavailable,
        title: TITLE,
        message: MESSAGE,
    }
}

pub const fn startup_fallback(code: ErrorCode) -> StartupFallback {
    match code {
        ErrorCode::WebviewUnavailable => webview_unavailable_fallback(),
        _ => StartupFallback {
            code: ErrorCode::InternalError,
            title: TITLE,
            message: INTERNAL_MESSAGE,
        },
    }
}

#[cfg(windows)]
pub fn show_webview_unavailable() {
    show_fallback(webview_unavailable_fallback());
}

#[cfg(not(windows))]
pub fn show_webview_unavailable() {}

#[cfg(windows)]
pub fn show_startup_failure(code: ErrorCode) {
    show_fallback(startup_fallback(code));
}

#[cfg(not(windows))]
pub fn show_startup_failure(_code: ErrorCode) {}

#[cfg(windows)]
fn show_fallback(fallback: StartupFallback) {
    use std::ptr;
    use windows_sys::Win32::UI::WindowsAndMessaging::{MB_ICONERROR, MB_OK, MessageBoxW};

    let title = wide_null(fallback.title);
    let message = wide_null(fallback.message);
    unsafe {
        MessageBoxW(
            ptr::null_mut(),
            message.as_ptr(),
            title.as_ptr(),
            MB_OK | MB_ICONERROR,
        );
    }
}

#[cfg(windows)]
fn wide_null(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn webview_fallback_is_static_specific_and_recoverable() {
        let fallback = webview_unavailable_fallback();

        assert_eq!(fallback.code, ErrorCode::WebviewUnavailable);
        assert_eq!(fallback.title, "桌面笔记");
        assert!(fallback.message.contains("Microsoft Edge WebView2 Runtime"));
        assert!(fallback.message.contains("WEBVIEW_UNAVAILABLE"));
        assert!(fallback.message.contains("安装或修复"));
    }

    #[test]
    fn unrelated_startup_error_uses_controlled_internal_fallback() {
        let fallback = startup_fallback(ErrorCode::DatabaseCorrupted);

        assert_eq!(fallback.code, ErrorCode::InternalError);
        assert!(fallback.message.contains("INTERNAL_ERROR"));
        assert!(!fallback.message.contains("DATABASE_CORRUPTED"));
    }
}
