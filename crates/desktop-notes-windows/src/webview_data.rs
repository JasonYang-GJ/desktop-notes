use std::{
    fs::{self, Metadata, OpenOptions},
    io::{self, Read, Write},
    path::{Component, Path, PathBuf},
};

#[cfg(windows)]
use std::os::windows::fs::{MetadataExt, OpenOptionsExt};

use desktop_notes_core::{ErrorCode, FoundationError};

const COMPLETION_MARKER: &[u8] = b"v1\n";
const COMPLETION_MARKER_PATH: &str = "control/webview-autofill-cleanup-v1.done";
#[cfg(windows)]
const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0000_0400;
#[cfg(windows)]
const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
const AUTOFILL_FILE_NAMES: [&str; 8] = [
    "Web Data",
    "Web Data-journal",
    "Web Data-wal",
    "Web Data-shm",
    "Web Data For Account",
    "Web Data For Account-journal",
    "Web Data For Account-wal",
    "Web Data For Account-shm",
];

fn data_root_unavailable() -> FoundationError {
    FoundationError::new(
        ErrorCode::DataRootUnavailable,
        "Desktop Notes could not prepare its local data storage.",
    )
}

pub fn app_local_data_root(identifier: &str) -> Result<PathBuf, FoundationError> {
    let mut components = Path::new(identifier).components();
    if !matches!(components.next(), Some(Component::Normal(_))) || components.next().is_some() {
        return Err(data_root_unavailable());
    }

    dirs::data_local_dir()
        .map(|root| root.join(identifier))
        .ok_or_else(data_root_unavailable)
}

pub fn cleanup_legacy_webview_autofill(app_local_data_root: &Path) -> Result<(), FoundationError> {
    ensure_app_root(app_local_data_root)?;

    let control = app_local_data_root.join("control");
    let marker = app_local_data_root.join(COMPLETION_MARKER_PATH);
    let marker_is_valid = read_completion_marker(&control, &marker)?;

    let webview = app_local_data_root.join("EBWebView");
    if existing_directory_is_safe(&webview)? {
        let webview_default = webview.join("Default");
        if existing_directory_is_safe(&webview_default)? {
            for file_name in AUTOFILL_FILE_NAMES {
                ensure_app_root(app_local_data_root)?;
                require_existing_directory_is_safe(&webview)?;
                require_existing_directory_is_safe(&webview_default)?;
                remove_expected_file(&webview_default.join(file_name))?;
            }
        }
    }

    if marker_is_valid {
        return Ok(());
    }

    ensure_app_root(app_local_data_root)?;
    ensure_control_directory(&control)?;
    require_existing_directory_is_safe(&control)?;
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(windows)]
    options.custom_flags(FILE_FLAG_OPEN_REPARSE_POINT);
    match options.open(&marker) {
        Ok(mut file) => {
            file.write_all(COMPLETION_MARKER)
                .and_then(|()| file.sync_all())
                .map_err(|_| data_root_unavailable())?;
        }
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
            if !read_completion_marker(&control, &marker)? {
                return Err(data_root_unavailable());
            }
        }
        Err(_) => return Err(data_root_unavailable()),
    }

    if !read_completion_marker(&control, &marker)? {
        return Err(data_root_unavailable());
    }
    Ok(())
}

fn ensure_app_root(path: &Path) -> Result<(), FoundationError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_dir() && !is_reparse_point(&metadata) => Ok(()),
        Ok(_) => Err(data_root_unavailable()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            fs::create_dir(path).map_err(|_| data_root_unavailable())?;
            require_existing_directory_is_safe(path)
        }
        Err(_) => Err(data_root_unavailable()),
    }
}

fn ensure_control_directory(path: &Path) -> Result<(), FoundationError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_dir() && !is_reparse_point(&metadata) => Ok(()),
        Ok(_) => Err(data_root_unavailable()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            fs::create_dir(path).map_err(|_| data_root_unavailable())?;
            require_existing_directory_is_safe(path)
        }
        Err(_) => Err(data_root_unavailable()),
    }
}

fn existing_directory_is_safe(path: &Path) -> Result<bool, FoundationError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_dir() && !is_reparse_point(&metadata) => Ok(true),
        Ok(_) => Err(data_root_unavailable()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(_) => Err(data_root_unavailable()),
    }
}

fn require_existing_directory_is_safe(path: &Path) -> Result<(), FoundationError> {
    if existing_directory_is_safe(path)? {
        Ok(())
    } else {
        Err(data_root_unavailable())
    }
}

fn read_completion_marker(control: &Path, marker: &Path) -> Result<bool, FoundationError> {
    if !existing_directory_is_safe(control)? {
        return Ok(false);
    }

    let metadata = match fs::symlink_metadata(marker) {
        Ok(metadata) if metadata.is_file() && !is_reparse_point(&metadata) => metadata,
        Ok(_) => return Err(data_root_unavailable()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(_) => return Err(data_root_unavailable()),
    };
    if is_reparse_point(&metadata) {
        return Err(data_root_unavailable());
    }

    require_existing_directory_is_safe(control)?;
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    options.custom_flags(FILE_FLAG_OPEN_REPARSE_POINT);
    let mut file = options.open(marker).map_err(|_| data_root_unavailable())?;
    let opened_metadata = file.metadata().map_err(|_| data_root_unavailable())?;
    if !opened_metadata.is_file() || is_reparse_point(&opened_metadata) {
        return Err(data_root_unavailable());
    }
    let mut contents = Vec::new();
    file.read_to_end(&mut contents)
        .map_err(|_| data_root_unavailable())?;
    require_existing_directory_is_safe(control)?;
    if contents == COMPLETION_MARKER {
        Ok(true)
    } else {
        Err(data_root_unavailable())
    }
}

#[cfg(windows)]
fn is_reparse_point(metadata: &Metadata) -> bool {
    metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

#[cfg(not(windows))]
fn is_reparse_point(metadata: &Metadata) -> bool {
    metadata.file_type().is_symlink()
}

fn remove_expected_file(path: &Path) -> Result<(), FoundationError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_file() && !is_reparse_point(&metadata) => {
            fs::remove_file(path).map_err(|_| data_root_unavailable())?;
            match fs::symlink_metadata(path) {
                Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
                _ => Err(data_root_unavailable()),
            }
        }
        Ok(_) => Err(data_root_unavailable()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err(data_root_unavailable()),
    }
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::{Path, PathBuf},
        time::{SystemTime, UNIX_EPOCH},
    };

    #[cfg(windows)]
    use std::os::windows::fs::symlink_dir;

    use desktop_notes_core::ErrorCode;

    use super::{COMPLETION_MARKER, cleanup_legacy_webview_autofill};

    struct TestRoot(PathBuf);

    impl TestRoot {
        fn new(label: &str) -> Self {
            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path = std::env::temp_dir().join(format!(
                "desktop-notes-webview-cleanup-{label}-{}-{nonce}",
                std::process::id()
            ));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TestRoot {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn autofill_files(root: &Path) -> Vec<PathBuf> {
        let default = root.join("EBWebView").join("Default");
        [
            "Web Data",
            "Web Data-journal",
            "Web Data-wal",
            "Web Data-shm",
            "Web Data For Account",
            "Web Data For Account-journal",
            "Web Data For Account-wal",
            "Web Data For Account-shm",
        ]
        .into_iter()
        .map(|name| default.join(name))
        .collect()
    }

    #[test]
    fn removes_only_legacy_autofill_files_and_is_idempotent() {
        let root = TestRoot::new("success");
        let encrypted_db = root.path().join("data").join("desktop-notes.db");
        fs::create_dir_all(encrypted_db.parent().unwrap()).unwrap();
        let encrypted_bytes = b"synthetic encrypted database bytes";
        fs::write(&encrypted_db, encrypted_bytes).unwrap();

        for file in autofill_files(root.path()) {
            fs::create_dir_all(file.parent().unwrap()).unwrap();
            fs::write(file, b"historical plaintext marker").unwrap();
        }

        cleanup_legacy_webview_autofill(root.path()).unwrap();
        let recreated_web_data = autofill_files(root.path()).remove(0);
        fs::write(&recreated_web_data, b"downgrade plaintext marker").unwrap();
        cleanup_legacy_webview_autofill(root.path()).unwrap();

        assert!(
            autofill_files(root.path())
                .iter()
                .all(|path| !path.exists())
        );
        assert_eq!(fs::read(encrypted_db).unwrap(), encrypted_bytes);
        assert_eq!(
            fs::read(root.path().join("control/webview-autofill-cleanup-v1.done")).unwrap(),
            b"v1\n"
        );
    }

    #[test]
    fn unexpected_autofill_directory_fails_closed_without_completion_marker() {
        let root = TestRoot::new("directory");
        let target = root.path().join("EBWebView/Default/Web Data");
        fs::create_dir_all(&target).unwrap();

        let error = cleanup_legacy_webview_autofill(root.path()).unwrap_err();

        assert_eq!(error.code(), ErrorCode::DataRootUnavailable);
        assert!(target.is_dir());
        assert!(
            !root
                .path()
                .join("control/webview-autofill-cleanup-v1.done")
                .exists()
        );
    }

    #[test]
    fn invalid_completion_marker_fails_closed() {
        let root = TestRoot::new("invalid-marker");
        let marker = root.path().join("control/webview-autofill-cleanup-v1.done");
        fs::create_dir_all(marker.parent().unwrap()).unwrap();
        fs::write(&marker, b"unexpected").unwrap();
        let autofill = root.path().join("EBWebView/Default/Web Data");
        fs::create_dir_all(autofill.parent().unwrap()).unwrap();
        fs::write(&autofill, b"must remain on invalid control state").unwrap();

        let error = cleanup_legacy_webview_autofill(root.path()).unwrap_err();

        assert_eq!(error.code(), ErrorCode::DataRootUnavailable);
        assert_eq!(
            fs::read(autofill).unwrap(),
            b"must remain on invalid control state"
        );
    }

    #[cfg(windows)]
    #[test]
    fn webview_ancestor_reparse_point_fails_closed_without_deleting_outside_file() {
        let root = TestRoot::new("webview-reparse-root");
        let outside = TestRoot::new("webview-reparse-outside");
        let outside_target = outside.path().join("Default/Web Data");
        fs::create_dir_all(outside_target.parent().unwrap()).unwrap();
        fs::write(&outside_target, b"outside decoy").unwrap();
        symlink_dir(outside.path(), root.path().join("EBWebView")).unwrap();

        let error = cleanup_legacy_webview_autofill(root.path()).unwrap_err();

        assert_eq!(error.code(), ErrorCode::DataRootUnavailable);
        assert_eq!(fs::read(outside_target).unwrap(), b"outside decoy");
        assert!(
            !root
                .path()
                .join("control/webview-autofill-cleanup-v1.done")
                .exists()
        );
    }

    #[cfg(windows)]
    #[test]
    fn control_ancestor_reparse_point_fails_closed_without_reading_outside_marker() {
        let root = TestRoot::new("control-reparse-root");
        let outside = TestRoot::new("control-reparse-outside");
        let outside_marker = outside.path().join("webview-autofill-cleanup-v1.done");
        fs::write(&outside_marker, COMPLETION_MARKER).unwrap();
        symlink_dir(outside.path(), root.path().join("control")).unwrap();

        let error = cleanup_legacy_webview_autofill(root.path()).unwrap_err();

        assert_eq!(error.code(), ErrorCode::DataRootUnavailable);
        assert_eq!(fs::read(outside_marker).unwrap(), COMPLETION_MARKER);
    }

    #[cfg(windows)]
    #[test]
    fn app_root_reparse_point_fails_closed_without_deleting_outside_file() {
        let parent = TestRoot::new("app-reparse-parent");
        let outside = TestRoot::new("app-reparse-outside");
        let outside_target = outside.path().join("EBWebView/Default/Web Data");
        fs::create_dir_all(outside_target.parent().unwrap()).unwrap();
        fs::write(&outside_target, b"outside decoy").unwrap();
        let linked_root = parent.path().join("app-root");
        symlink_dir(outside.path(), &linked_root).unwrap();

        let error = cleanup_legacy_webview_autofill(&linked_root).unwrap_err();

        assert_eq!(error.code(), ErrorCode::DataRootUnavailable);
        assert_eq!(fs::read(outside_target).unwrap(), b"outside decoy");
    }
}
