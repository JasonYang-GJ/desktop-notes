use std::{fs, path::Path};

fn workspace_root() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .unwrap()
        .to_path_buf()
}

fn read_tree(root: &Path, extensions: &[&str]) -> String {
    let mut output = String::new();
    visit(root, extensions, &mut output);
    output
}

fn visit(root: &Path, extensions: &[&str], output: &mut String) {
    for entry in fs::read_dir(root).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            visit(&path, extensions, output);
        } else if path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extensions.contains(&extension))
        {
            output.push_str(&fs::read_to_string(path).unwrap());
            output.push('\n');
        }
    }
}

#[test]
fn core_manifest_has_no_platform_ui_or_database_dependencies() {
    let root = workspace_root();
    let manifest = fs::read_to_string(root.join("crates/desktop-notes-core/Cargo.toml"))
        .unwrap()
        .to_ascii_lowercase();
    for forbidden in ["tauri", "windows-sys", "rusqlite", "tiptap"] {
        assert!(
            !manifest.contains(forbidden),
            "desktop-notes-core contains forbidden dependency {forbidden}"
        );
    }
}

#[test]
fn windows_adapter_contains_no_business_sql() {
    let source = read_tree(
        &workspace_root().join("crates/desktop-notes-windows/src"),
        &["rs"],
    )
    .to_ascii_lowercase();
    for forbidden in [
        "rusqlite",
        "create table",
        "insert into",
        "select ",
        "pragma ",
    ] {
        assert!(
            !source.contains(forbidden),
            "Windows adapter contains forbidden SQL boundary: {forbidden}"
        );
    }
}

#[test]
fn tauri_commands_contain_no_sql_and_match_the_audited_allowlist() {
    let root = workspace_root();
    let host = read_tree(&root.join("apps/desktop/src-tauri/src"), &["rs"]);
    let lowered = host.to_ascii_lowercase();
    for forbidden in [
        "rusqlite",
        "create table",
        "insert into",
        "select ",
        "pragma ",
    ] {
        assert!(
            !lowered.contains(forbidden),
            "Tauri command layer contains forbidden SQL: {forbidden}"
        );
    }
    // B01-B10 typed commands are declared directly; the bounded image operations use Tauri's
    // async command pool so decode/decrypt/delete cannot block the WebView thread. B05's nine
    // organization commands share one macro declaration. B10 adds four narrow window-shell
    // commands and does not expose a generic window API.
    assert_eq!(host.matches("#[tauri::command]").count(), 20);
    assert_eq!(host.matches("#[tauri::command(async)]").count(), 3);
    assert_eq!(host.matches("organization_command!(").count(), 9);
    for command in [
        "get_foundation_status",
        "create_note",
        "get_note",
        "delete_note",
        "list_notes_for_date",
        "update_note_content",
        "list_note_counts_for_month",
        "change_note_date",
        "undo_note_date_change",
        "list_tags",
        "create_tag",
        "rename_tag",
        "delete_tag",
        "list_tags_for_note",
        "assign_tag",
        "remove_tag",
        "set_note_pinned",
        "list_recent_notes",
        "search_notes",
        "import_image_asset",
        "read_image_asset",
        "discard_image_asset",
        "take_quick_capture",
        "finish_quick_capture",
        "get_shortcut_config",
        "set_shortcut_config",
        "get_backup_status",
        "get_window_session",
        "set_visual_state",
        "set_theme_mode",
        "save_window_placement",
    ] {
        assert!(
            host.contains(&format!("fn {command}("))
                || host.contains(&format!("organization_command!({command},")),
            "missing typed Tauri command boundary for {command}"
        );
        assert!(host.contains(&format!("\"{command}\"")));
    }
    for forbidden in ["execute_sql", "read_file", "generic_shell", "generic_fs"] {
        assert!(!lowered.contains(forbidden));
    }
}

#[test]
fn frontend_has_no_direct_system_or_network_escape_hatches() {
    let root = workspace_root();
    let frontend =
        read_tree(&root.join("apps/desktop/src"), &["ts", "tsx", "css"]).to_ascii_lowercase();
    for forbidden in [
        "@tauri-apps/plugin-fs",
        "@tauri-apps/plugin-shell",
        "@tauri-apps/plugin-sql",
        "@tauri-apps/plugin-clipboard",
        "fetch(",
        "xmlhttprequest",
        "websocket(",
        "eventsource(",
        "sendbeacon(",
        "from \"ajv",
        "from 'ajv",
        "new function(",
    ] {
        assert!(
            !frontend.contains(forbidden),
            "Frontend contains forbidden direct capability: {forbidden}"
        );
    }
}

#[test]
fn tauri_capability_and_csp_are_local_and_minimal() {
    let root = workspace_root();
    let capability =
        fs::read_to_string(root.join("apps/desktop/src-tauri/capabilities/default.json"))
            .unwrap()
            .to_ascii_lowercase();
    assert!(capability.contains("core:default"));
    for forbidden in [
        "plugin:fs",
        "plugin:shell",
        "plugin:sql",
        "plugin:http",
        "clipboard",
    ] {
        assert!(!capability.contains(forbidden));
    }

    let config = fs::read_to_string(root.join("apps/desktop/src-tauri/tauri.conf.json"))
        .unwrap()
        .to_ascii_lowercase();
    for directive in [
        "default-src 'self'",
        "script-src 'self'",
        "connect-src 'self' ipc: http://ipc.localhost",
        "object-src 'none'",
        "frame-src 'none'",
    ] {
        assert!(
            config.contains(directive),
            "CSP directive missing: {directive}"
        );
    }
    assert!(!config.contains("script-src 'self' http"));
    assert!(!config.contains("connect-src 'self' http"));
    assert!(
        config.contains("\"dragdropenabled\": false"),
        "Windows HTML5 drag/drop must not be replaced by Tauri's native file-drop handler"
    );
    assert!(
        config.contains("\"generalautofillenabled\": false"),
        "WebView2 general autofill must not retain Note or Tag input in its Web Data store"
    );

    let manifest = fs::read_to_string(root.join("apps/desktop/src-tauri/Cargo.toml")).unwrap();
    assert!(manifest.contains("default = [\"custom-protocol\"]"));
    assert!(manifest.contains("custom-protocol = [\"tauri/custom-protocol\"]"));
}

#[test]
fn startup_failure_has_a_native_webview_fallback_and_nonzero_exit() {
    let root = workspace_root();
    let main = fs::read_to_string(root.join("apps/desktop/src-tauri/src/main.rs")).unwrap();
    let host = fs::read_to_string(root.join("apps/desktop/src-tauri/src/lib.rs")).unwrap();
    let windows = read_tree(&root.join("crates/desktop-notes-windows/src"), &["rs"]);

    assert!(host.contains("pub fn run() -> Result<(), FoundationError>"));
    assert!(host.contains("ensure_webview_runtime()?"));
    assert!(host.contains("app_local_data_root(&context.config().identifier)?"));
    assert!(host.contains("cleanup_legacy_webview_autofill(&app_local_data_root)?"));
    let cleanup_position = host
        .find("cleanup_legacy_webview_autofill(&app_local_data_root)?")
        .unwrap();
    let builder_position = host.find("tauri::Builder::default()").unwrap();
    assert!(
        cleanup_position < builder_position,
        "legacy WebView autofill data must be removed before Tauri can create a WebView"
    );
    assert!(host.contains("tauri::webview_version()"));
    assert!(host.contains("tauri_runtime::Error::WebviewRuntimeNotInstalled"));
    assert!(host.contains("ErrorCode::WebviewUnavailable"));
    assert!(main.contains("show_startup_failure(error.code())"));
    assert!(main.contains("std::process::exit(1)"));
    assert!(windows.contains("MessageBoxW"));
    assert!(windows.contains("WEBVIEW_UNAVAILABLE"));
}

#[test]
fn sensitive_sql_and_exclusive_destination_guards_are_present() {
    let root = workspace_root();
    let sqlcipher =
        fs::read_to_string(root.join("crates/desktop-notes-infra/src/sqlcipher.rs")).unwrap();
    let keyring =
        fs::read_to_string(root.join("crates/desktop-notes-windows/src/keyring.rs")).unwrap();

    assert!(!sqlcipher.contains("let sql = format!(\"PRAGMA key"));
    assert!(sqlcipher.contains("Zeroizing::new(String::with_capacity"));
    assert!(sqlcipher.contains("struct ExclusiveDestination"));
    assert!(keyring.contains("struct ExclusiveDestination"));
}
