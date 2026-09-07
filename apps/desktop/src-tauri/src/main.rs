#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    if let Err(error) = desktop_notes_desktop::run() {
        desktop_notes_windows::show_startup_failure(error.code());
        std::process::exit(1);
    }
}
