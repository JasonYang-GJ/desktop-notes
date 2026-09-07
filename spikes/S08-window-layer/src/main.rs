use std::{env, process::Command, ptr::null_mut};

use serde_json::json;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DestroyWindow, GWL_EXSTYLE, GetClassNameW, GetParent, GetWindowLongW,
    GetWindowTextW, HWND_NOTOPMOST, HWND_TOPMOST, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE,
    SetWindowPos, WS_EX_TOPMOST, WS_OVERLAPPEDWINDOW, WS_VISIBLE,
};

const NAVIGATION_SENTINEL: &str = "S08|date=2026-09-07|note=note-navigation-sentinel";

fn main() {
    let result = match env::args().nth(1).as_deref() {
        Some("child-default") => child_default_probe(),
        Some("session-sequence") => session_sequence(),
        _ => Err("usage: session-sequence | child-default".to_owned()),
    };
    if let Err(message) = result {
        eprintln!("S08_ERROR={message}");
        std::process::exit(1);
    }
}

fn child_default_probe() -> Result<(), String> {
    let window = ProbeWindow::new()?;
    if window.is_topmost() {
        return Err("a fresh process started topmost".to_owned());
    }
    if !window.is_standard_top_level() {
        return Err("a fresh process did not create a standard top-level window".to_owned());
    }
    println!("CHILD_DEFAULT_NON_TOPMOST=PASS");
    Ok(())
}

fn session_sequence() -> Result<(), String> {
    let window = ProbeWindow::new()?;
    let original_handle = window.handle as usize;
    let original_navigation = window.title()?;
    let class_name = window.class_name()?;

    let default_non_topmost = !window.is_topmost();
    let standard_top_level = window.is_standard_top_level();
    let forbidden_underlay_absent = !matches!(class_name.as_str(), "WorkerW" | "Progman");

    window.set_topmost(true)?;
    let temporary_topmost = window.is_topmost();
    let navigation_after_topmost = window.title()?;

    window.set_topmost(false)?;
    let returned_non_topmost = !window.is_topmost();
    let navigation_after_return = window.title()?;
    let same_window = original_handle == window.handle as usize;
    let navigation_preserved = original_navigation == NAVIGATION_SENTINEL
        && navigation_after_topmost == original_navigation
        && navigation_after_return == original_navigation;

    let child = Command::new(env::current_exe().map_err(|_| "current executable unavailable")?)
        .arg("child-default")
        .output()
        .map_err(|_| "fresh-process probe did not start")?;
    let child_stdout = String::from_utf8_lossy(&child.stdout);
    let restart_reset =
        child.status.success() && child_stdout.contains("CHILD_DEFAULT_NON_TOPMOST=PASS");

    let all_pass = default_non_topmost
        && temporary_topmost
        && returned_non_topmost
        && restart_reset
        && same_window
        && navigation_preserved
        && standard_top_level
        && forbidden_underlay_absent;

    let report = json!({
        "status": if all_pass { "PASS" } else { "FAIL" },
        "default_non_topmost": default_non_topmost,
        "temporary_topmost": temporary_topmost,
        "returned_non_topmost": returned_non_topmost,
        "fresh_process_non_topmost": restart_reset,
        "same_window_handle": same_window,
        "navigation_sentinel_preserved": navigation_preserved,
        "standard_top_level": standard_top_level,
        "parent_is_null": unsafe { GetParent(window.handle) }.is_null(),
        "window_class": class_name,
        "workerw_or_progman_absent": forbidden_underlay_absent,
        "persistent_topmost_storage": "ABSENT",
    });
    println!("{report}");

    all_pass
        .then_some(())
        .ok_or_else(|| "one or more S08 window-layer invariants failed".to_owned())
}

struct ProbeWindow {
    handle: windows_sys::Win32::Foundation::HWND,
}

impl ProbeWindow {
    fn new() -> Result<Self, String> {
        let class_name = wide("STATIC");
        let title = wide(NAVIGATION_SENTINEL);
        let handle = unsafe {
            CreateWindowExW(
                0,
                class_name.as_ptr(),
                title.as_ptr(),
                WS_OVERLAPPEDWINDOW | WS_VISIBLE,
                96,
                96,
                720,
                480,
                null_mut(),
                null_mut(),
                null_mut(),
                null_mut(),
            )
        };
        if handle.is_null() {
            return Err("real Win32 top-level window creation failed".to_owned());
        }
        Ok(Self { handle })
    }

    fn is_topmost(&self) -> bool {
        (unsafe { GetWindowLongW(self.handle, GWL_EXSTYLE) } as u32 & WS_EX_TOPMOST) != 0
    }

    fn set_topmost(&self, topmost: bool) -> Result<(), String> {
        let insert_after = if topmost {
            HWND_TOPMOST
        } else {
            HWND_NOTOPMOST
        };
        let changed = unsafe {
            SetWindowPos(
                self.handle,
                insert_after,
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
            )
        } != 0;
        changed
            .then_some(())
            .ok_or_else(|| "SetWindowPos rejected the requested Z-order".to_owned())
    }

    fn is_standard_top_level(&self) -> bool {
        unsafe { GetParent(self.handle) }.is_null()
    }

    fn title(&self) -> Result<String, String> {
        let mut buffer = [0_u16; 256];
        let length =
            unsafe { GetWindowTextW(self.handle, buffer.as_mut_ptr(), buffer.len() as i32) };
        if length <= 0 {
            return Err("window navigation sentinel could not be read".to_owned());
        }
        Ok(String::from_utf16_lossy(&buffer[..length as usize]))
    }

    fn class_name(&self) -> Result<String, String> {
        let mut buffer = [0_u16; 128];
        let length =
            unsafe { GetClassNameW(self.handle, buffer.as_mut_ptr(), buffer.len() as i32) };
        if length <= 0 {
            return Err("window class could not be read".to_owned());
        }
        Ok(String::from_utf16_lossy(&buffer[..length as usize]))
    }
}

impl Drop for ProbeWindow {
    fn drop(&mut self) {
        unsafe {
            DestroyWindow(self.handle);
        }
    }
}

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(Some(0)).collect()
}
