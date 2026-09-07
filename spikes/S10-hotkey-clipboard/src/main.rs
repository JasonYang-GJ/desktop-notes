use std::{
    env,
    io::{BufRead, BufReader, Write},
    process::{Command, Stdio},
    ptr::null_mut,
    thread,
    time::{Duration, Instant},
};

use desktop_notes_core::{CapturedClipboard, ClipboardReader};
use desktop_notes_windows::WindowsClipboard;
use windows_sys::Win32::{
    Foundation::GlobalFree,
    System::{
        DataExchange::{CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData},
        Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalUnlock},
        Ole::{CF_DIB, CF_HDROP, CF_UNICODETEXT},
    },
    UI::Input::KeyboardAndMouse::{
        MOD_ALT, MOD_CONTROL, MOD_NOREPEAT, RegisterHotKey, UnregisterHotKey,
    },
    UI::WindowsAndMessaging::{CreateWindowExW, DestroyWindow},
};

const HOTKEY_ID: i32 = 0x444e_1001;
const VK_N: u32 = b'N' as u32;

fn main() {
    let result = match env::args().nth(1).as_deref() {
        Some("hotkey-probe") => hotkey_probe(),
        Some("hotkey-hold") => hotkey_hold(),
        Some("hotkey-hold-q") => hotkey_hold_key(b'Q' as u32),
        Some("hotkey-conflict") => hotkey_conflict(),
        Some("clipboard-hold") => clipboard_hold(),
        Some("clipboard-matrix") => clipboard_matrix(),
        _ => Err("usage: hotkey-probe | hotkey-hold | hotkey-conflict | clipboard-matrix"),
    };
    if let Err(message) = result {
        eprintln!("S10_ERROR={message}");
        std::process::exit(1);
    }
}

fn hotkey_probe() -> Result<(), &'static str> {
    register_default()?;
    unregister_default()?;
    register_default()?;
    unregister_default()?;
    println!("DEFAULT_REGISTER=PASS");
    println!("UNREGISTER_REREGISTER=PASS");
    Ok(())
}

fn hotkey_hold() -> Result<(), &'static str> {
    hotkey_hold_key(VK_N)
}

fn hotkey_hold_key(key: u32) -> Result<(), &'static str> {
    let registered = unsafe {
        RegisterHotKey(
            null_mut(),
            HOTKEY_ID,
            MOD_CONTROL | MOD_ALT | MOD_NOREPEAT,
            key,
        )
    } != 0;
    if !registered {
        return Err("requested test hotkey registration failed");
    }
    println!("HOTKEY_READY");
    thread::sleep(Duration::from_secs(45));
    unregister_default()?;
    Ok(())
}

fn hotkey_conflict() -> Result<(), &'static str> {
    let registered = unsafe {
        RegisterHotKey(
            null_mut(),
            HOTKEY_ID + 1,
            MOD_CONTROL | MOD_ALT | MOD_NOREPEAT,
            VK_N,
        )
    } != 0;
    if registered {
        unsafe {
            UnregisterHotKey(null_mut(), HOTKEY_ID + 1);
        }
        return Err("occupied Ctrl+Alt+N was registered twice");
    }
    println!("CONFLICT_DETECTED=PASS");
    Ok(())
}

fn register_default() -> Result<(), &'static str> {
    let registered = unsafe {
        RegisterHotKey(
            null_mut(),
            HOTKEY_ID,
            MOD_CONTROL | MOD_ALT | MOD_NOREPEAT,
            VK_N,
        )
    } != 0;
    registered
        .then_some(())
        .ok_or("Ctrl+Alt+N registration failed")
}

fn unregister_default() -> Result<(), &'static str> {
    (unsafe { UnregisterHotKey(null_mut(), HOTKEY_ID) } != 0)
        .then_some(())
        .ok_or("Ctrl+Alt+N unregister failed")
}

fn clipboard_matrix() -> Result<(), &'static str> {
    set_clipboard(&[(u32::from(CF_UNICODETEXT), utf16_bytes("S10 synthetic text"))])?;
    assert_kind("text", WindowsClipboard::default().read_once(), 1, 4)?;

    set_clipboard(&[(u32::from(CF_DIB), one_pixel_dib())])?;
    assert_kind("image", WindowsClipboard::default().read_once(), 1, 4)?;

    set_clipboard(&[
        (
            u32::from(CF_UNICODETEXT),
            utf16_bytes("S10 synthetic combined"),
        ),
        (u32::from(CF_DIB), one_pixel_dib()),
    ])?;
    assert_kind(
        "text_and_image",
        WindowsClipboard::default().read_once(),
        1,
        4,
    )?;

    clear_clipboard()?;
    assert_kind("empty", WindowsClipboard::default().read_once(), 1, 4)?;

    set_clipboard(&[(u32::from(CF_HDROP), vec![0_u8; 24])])?;
    assert_kind("unsupported", WindowsClipboard::default().read_once(), 1, 4)?;

    set_clipboard(&[(u32::from(CF_UNICODETEXT), utf16_bytes("S10 locked fixture"))])?;
    let executable = env::current_exe().map_err(|_| "clipboard holder path unavailable")?;
    let mut holder = Command::new(executable)
        .arg("clipboard-hold")
        .stdout(Stdio::piped())
        .spawn()
        .map_err(|_| "clipboard holder process did not start")?;
    let mut ready = String::new();
    BufReader::new(
        holder
            .stdout
            .take()
            .ok_or("clipboard holder output unavailable")?,
    )
    .read_line(&mut ready)
    .map_err(|_| "clipboard holder readiness unavailable")?;
    if ready.trim() != "CLIPBOARD_LOCK_READY" {
        return Err("clipboard holder could not lock the transaction");
    }
    let started = Instant::now();
    let busy = WindowsClipboard::default().read_once();
    let elapsed = started.elapsed();
    if !holder
        .wait()
        .map_err(|_| "clipboard holder process failed")?
        .success()
    {
        return Err("clipboard holder process exited unsuccessfully");
    }
    println!(
        "BUSY_OBSERVED kind={} attempts={} elapsed_ms={}",
        busy.content.kind(),
        busy.attempts,
        elapsed.as_millis()
    );
    assert_kind("busy", busy, 4, 4)?;
    if elapsed > Duration::from_millis(100) {
        return Err("busy acquisition exceeded its bounded retry window");
    }
    clear_clipboard()?;
    println!("CLIPBOARD_MATRIX=PASS");
    println!("ONE_TRANSACTION_BOUNDED_RETRY=PASS");
    Ok(())
}

fn clipboard_hold() -> Result<(), &'static str> {
    let class_name: Vec<u16> = "STATIC\0".encode_utf16().collect();
    let window = unsafe {
        CreateWindowExW(
            0,
            class_name.as_ptr(),
            class_name.as_ptr(),
            0,
            0,
            0,
            0,
            0,
            null_mut(),
            null_mut(),
            null_mut(),
            null_mut(),
        )
    };
    if window.is_null() {
        return Err("clipboard holder window creation failed");
    }
    if unsafe { OpenClipboard(window) } == 0 {
        unsafe { DestroyWindow(window) };
        return Err("clipboard holder could not open clipboard");
    }
    println!("CLIPBOARD_LOCK_READY");
    std::io::stdout()
        .flush()
        .map_err(|_| "clipboard holder readiness flush failed")?;
    thread::sleep(Duration::from_millis(160));
    unsafe {
        CloseClipboard();
        DestroyWindow(window);
    }
    Ok(())
}

fn assert_kind(
    expected: &str,
    actual: desktop_notes_core::ClipboardRead,
    minimum_attempts: u8,
    maximum_attempts: u8,
) -> Result<(), &'static str> {
    if actual.content.kind() != expected {
        return Err("clipboard content kind mismatch");
    }
    if !(minimum_attempts..=maximum_attempts).contains(&actual.attempts) {
        return Err("clipboard acquisition attempt count mismatch");
    }
    match actual.content {
        CapturedClipboard::Text(text) if expected == "text" && text != "S10 synthetic text" => {
            return Err("text fixture mismatch");
        }
        CapturedClipboard::ImagePng(bytes) if expected == "image" && bytes.is_empty() => {
            return Err("image fixture normalized to empty payload");
        }
        CapturedClipboard::TextAndImage { text, png }
            if expected == "text_and_image"
                && (text != "S10 synthetic combined" || png.is_empty()) =>
        {
            return Err("combined fixture mismatch");
        }
        _ => {}
    }
    println!(
        "{}=PASS attempts={}",
        expected.to_ascii_uppercase(),
        actual.attempts
    );
    Ok(())
}

fn clear_clipboard() -> Result<(), &'static str> {
    open_clipboard()?;
    let cleared = unsafe { EmptyClipboard() } != 0;
    unsafe {
        CloseClipboard();
    }
    cleared.then_some(()).ok_or("clipboard clear failed")
}

fn set_clipboard(formats: &[(u32, Vec<u8>)]) -> Result<(), &'static str> {
    open_clipboard()?;
    if unsafe { EmptyClipboard() } == 0 {
        unsafe { CloseClipboard() };
        return Err("clipboard reset failed");
    }
    for (format, bytes) in formats {
        let memory = unsafe { GlobalAlloc(GMEM_MOVEABLE, bytes.len()) };
        if memory.is_null() {
            unsafe { CloseClipboard() };
            return Err("clipboard allocation failed");
        }
        let pointer = unsafe { GlobalLock(memory) }.cast::<u8>();
        if pointer.is_null() {
            unsafe {
                GlobalFree(memory);
                CloseClipboard();
            }
            return Err("clipboard memory lock failed");
        }
        unsafe {
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), pointer, bytes.len());
            GlobalUnlock(memory);
        }
        if unsafe { SetClipboardData(*format, memory) }.is_null() {
            unsafe {
                GlobalFree(memory);
                CloseClipboard();
            }
            return Err("clipboard format publication failed");
        }
    }
    unsafe {
        CloseClipboard();
    }
    Ok(())
}

fn open_clipboard() -> Result<(), &'static str> {
    for _ in 0..20 {
        if unsafe { OpenClipboard(null_mut()) } != 0 {
            return Ok(());
        }
        thread::sleep(Duration::from_millis(10));
    }
    Err("clipboard could not be opened for the synthetic fixture")
}

fn utf16_bytes(value: &str) -> Vec<u8> {
    value
        .encode_utf16()
        .chain([0])
        .flat_map(u16::to_le_bytes)
        .collect()
}

fn one_pixel_dib() -> Vec<u8> {
    let mut dib = vec![0_u8; 44];
    dib[0..4].copy_from_slice(&40_u32.to_le_bytes());
    dib[4..8].copy_from_slice(&1_i32.to_le_bytes());
    dib[8..12].copy_from_slice(&1_i32.to_le_bytes());
    dib[12..14].copy_from_slice(&1_u16.to_le_bytes());
    dib[14..16].copy_from_slice(&32_u16.to_le_bytes());
    dib[20..24].copy_from_slice(&4_u32.to_le_bytes());
    dib[40..44].copy_from_slice(&[0, 80, 160, 255]);
    dib
}
