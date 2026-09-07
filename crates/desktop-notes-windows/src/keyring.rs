use std::{
    fs::{self, File, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
};

use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use desktop_notes_core::{ErrorCode, FoundationError, KeyProtector, RootKey};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use zeroize::{Zeroize, Zeroizing};

const KEYRING_VERSION: u16 = 1;
const ACTIVE_KEY_ID: &str = "root-v1";
const PROTECTION: &str = "dpapi-current-user";
const INNER_MAGIC: &[u8; 4] = b"DNRK";
const INNER_DOMAIN: &[u8] = b"DesktopNotes/Product/root-inner/v1";
const DPAPI_ENTROPY: &[u8] = b"DesktopNotes/Product/DPAPI/v1";
const ROOT_LENGTH: usize = 32;
const INNER_LENGTH: usize = 4 + 2 + 2 + ROOT_LENGTH + 32;
const MAX_KEYRING_LENGTH: u64 = 128 * 1024;
const MAX_DPAPI_BLOB_LENGTH: usize = 64 * 1024;

#[derive(Debug)]
pub struct DpapiKeyring {
    path: PathBuf,
}

impl DpapiKeyring {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn export_wrapped_root(&self) -> Result<Vec<u8>, FoundationError> {
        let bytes = fs::read(&self.path).map_err(map_keyring_io)?;
        let document: KeyringDocument =
            serde_json::from_slice(&bytes).map_err(|_| key_unavailable())?;
        if document.version != KEYRING_VERSION || document.active_key_id != ACTIVE_KEY_ID {
            return Err(key_unavailable());
        }
        let slot = document
            .slots
            .iter()
            .find(|slot| slot.key_id == document.active_key_id)
            .ok_or_else(key_unavailable)?;
        if slot.protection != PROTECTION {
            return Err(key_unavailable());
        }
        let protected = BASE64
            .decode(&slot.wrapped_root_b64)
            .map_err(|_| key_unavailable())?;
        if protected.is_empty() || protected.len() > MAX_DPAPI_BLOB_LENGTH {
            return Err(key_unavailable());
        }
        let digest = Sha256::digest(&protected);
        if !constant_time_eq(slot.wrapped_root_sha256.as_bytes(), hex(&digest).as_bytes()) {
            return Err(key_unavailable());
        }
        Ok(protected)
    }

    pub fn unprotect_backup_root(protected: &[u8]) -> Result<RootKey, FoundationError> {
        if protected.is_empty() || protected.len() > MAX_DPAPI_BLOB_LENGTH {
            return Err(key_unavailable());
        }
        let mut inner = Zeroizing::new(dpapi_unprotect(protected)?);
        decode_inner(inner.as_mut_slice())
    }
}

impl KeyProtector for DpapiKeyring {
    fn load_key(&self) -> Result<Option<RootKey>, FoundationError> {
        let metadata = match fs::metadata(&self.path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(map_keyring_io(error)),
        };
        if !metadata.is_file() || metadata.len() == 0 || metadata.len() > MAX_KEYRING_LENGTH {
            return Err(key_unavailable());
        }

        let protected = self.export_wrapped_root()?;
        Self::unprotect_backup_root(&protected).map(Some)
    }

    fn create_and_store_key(&self) -> Result<RootKey, FoundationError> {
        if self.path.exists() {
            return Err(key_unavailable());
        }
        let parent = self.path.parent().ok_or_else(data_root_unavailable)?;
        fs::create_dir_all(parent).map_err(map_keyring_io)?;

        let mut root_bytes = [0_u8; ROOT_LENGTH];
        getrandom::fill(&mut root_bytes).map_err(|_| {
            FoundationError::new(
                ErrorCode::KeyUnavailable,
                "Windows could not generate the local encryption key.",
            )
        })?;
        let mut inner = Zeroizing::new(encode_inner(&root_bytes));
        let protected = dpapi_protect(inner.as_slice())?;
        inner.zeroize();

        let document = KeyringDocument {
            version: KEYRING_VERSION,
            active_key_id: ACTIVE_KEY_ID.to_owned(),
            slots: vec![KeySlot {
                key_id: ACTIVE_KEY_ID.to_owned(),
                protection: PROTECTION.to_owned(),
                wrapped_root_b64: BASE64.encode(&protected),
                wrapped_root_sha256: hex(&Sha256::digest(&protected)),
            }],
        };
        let encoded = serde_json::to_vec_pretty(&document).map_err(|_| {
            FoundationError::new(
                ErrorCode::InternalError,
                "The keyring could not be encoded.",
            )
        })?;
        let temporary = temporary_path(parent)?;
        let write_result = (|| {
            let mut file = OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&temporary)
                .map_err(map_keyring_io)?;
            file.write_all(&encoded).map_err(map_keyring_io)?;
            file.sync_all().map_err(map_keyring_io)?;
            drop(file);
            if self.path.exists() {
                return Err(key_unavailable());
            }
            commit_keyring_document(&temporary, &self.path, &encoded)
        })();
        if let Err(error) = write_result {
            let _ = fs::remove_file(&temporary);
            root_bytes.zeroize();
            return Err(error);
        }

        let root = RootKey::from_bytes(root_bytes);
        root_bytes.zeroize();
        Ok(root)
    }
}

fn commit_keyring_document(
    temporary: &Path,
    destination: &Path,
    encoded: &[u8],
) -> Result<(), FoundationError> {
    commit_keyring_document_with(temporary, destination, encoded, |source, destination| {
        fs::hard_link(source, destination)
    })
}

fn commit_keyring_document_with<Link>(
    temporary: &Path,
    destination: &Path,
    encoded: &[u8],
    link: Link,
) -> Result<(), FoundationError>
where
    Link: FnOnce(&Path, &Path) -> io::Result<()>,
{
    match link(temporary, destination) {
        Ok(()) => {
            let _ = fs::remove_file(temporary);
            return Ok(());
        }
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
            return Err(key_unavailable());
        }
        Err(_) => {}
    }

    persist_keyring_document(destination, encoded)?;
    let _ = fs::remove_file(temporary);
    Ok(())
}

fn persist_keyring_document(destination: &Path, encoded: &[u8]) -> Result<(), FoundationError> {
    persist_keyring_document_with(destination, encoded, |file, bytes| file.write_all(bytes))
}

fn persist_keyring_document_with<WriteAction>(
    destination: &Path,
    encoded: &[u8],
    write_action: WriteAction,
) -> Result<(), FoundationError>
where
    WriteAction: FnOnce(&mut File, &[u8]) -> io::Result<()>,
{
    let mut exclusive = ExclusiveDestination::create(destination).map_err(|error| {
        if error.kind() == io::ErrorKind::AlreadyExists {
            key_unavailable()
        } else {
            map_keyring_io(error)
        }
    })?;
    write_action(exclusive.file_mut(), encoded).map_err(map_keyring_io)?;
    exclusive.sync_close_and_verify(encoded)
}

struct ExclusiveDestination<'a> {
    path: &'a Path,
    file: Option<File>,
    committed: bool,
}

impl<'a> ExclusiveDestination<'a> {
    fn create(path: &'a Path) -> io::Result<Self> {
        let file = OpenOptions::new().create_new(true).write(true).open(path)?;
        Ok(Self {
            path,
            file: Some(file),
            committed: false,
        })
    }

    fn file_mut(&mut self) -> &mut File {
        self.file.as_mut().expect("exclusive file remains open")
    }

    fn sync_close_and_verify(self, expected: &[u8]) -> Result<(), FoundationError> {
        self.sync_close_and_verify_with(expected, |file| file.sync_all(), |path| fs::read(path))
    }

    fn sync_close_and_verify_with<SyncAction, ReadAction>(
        mut self,
        expected: &[u8],
        sync_action: SyncAction,
        read_action: ReadAction,
    ) -> Result<(), FoundationError>
    where
        SyncAction: FnOnce(&File) -> io::Result<()>,
        ReadAction: FnOnce(&Path) -> io::Result<Vec<u8>>,
    {
        let file = self.file.take().expect("exclusive file remains open");
        sync_action(&file).map_err(map_keyring_io)?;
        drop(file);
        let persisted = read_action(self.path).map_err(map_keyring_io)?;
        if !constant_time_eq(&persisted, expected) {
            return Err(data_root_unavailable());
        }
        self.committed = true;
        Ok(())
    }
}

impl Drop for ExclusiveDestination<'_> {
    fn drop(&mut self) {
        drop(self.file.take());
        if !self.committed {
            let _ = fs::remove_file(self.path);
        }
    }
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct KeyringDocument {
    version: u16,
    active_key_id: String,
    slots: Vec<KeySlot>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct KeySlot {
    key_id: String,
    protection: String,
    wrapped_root_b64: String,
    wrapped_root_sha256: String,
}

fn temporary_path(parent: &Path) -> Result<PathBuf, FoundationError> {
    let mut suffix = [0_u8; 16];
    getrandom::fill(&mut suffix).map_err(|_| key_unavailable())?;
    Ok(parent.join(format!(".keyring-{}.tmp", hex(&suffix))))
}

fn encode_inner(root: &[u8; ROOT_LENGTH]) -> Vec<u8> {
    let mut inner = Vec::with_capacity(INNER_LENGTH);
    inner.extend_from_slice(INNER_MAGIC);
    inner.extend_from_slice(&KEYRING_VERSION.to_le_bytes());
    inner.extend_from_slice(&(ROOT_LENGTH as u16).to_le_bytes());
    inner.extend_from_slice(root);
    let mut digest = Sha256::new();
    digest.update(INNER_DOMAIN);
    digest.update(&inner);
    inner.extend_from_slice(&digest.finalize());
    inner
}

fn decode_inner(inner: &mut [u8]) -> Result<RootKey, FoundationError> {
    if inner.len() != INNER_LENGTH
        || &inner[..4] != INNER_MAGIC
        || u16::from_le_bytes([inner[4], inner[5]]) != KEYRING_VERSION
        || usize::from(u16::from_le_bytes([inner[6], inner[7]])) != ROOT_LENGTH
    {
        return Err(key_unavailable());
    }
    let mut digest = Sha256::new();
    digest.update(INNER_DOMAIN);
    digest.update(&inner[..8 + ROOT_LENGTH]);
    if !constant_time_eq(&digest.finalize(), &inner[8 + ROOT_LENGTH..]) {
        return Err(key_unavailable());
    }
    let mut root = [0_u8; ROOT_LENGTH];
    root.copy_from_slice(&inner[8..8 + ROOT_LENGTH]);
    let protected_root = RootKey::from_bytes(root);
    root.zeroize();
    Ok(protected_root)
}

#[cfg(windows)]
fn dpapi_protect(plaintext: &[u8]) -> Result<Vec<u8>, FoundationError> {
    use std::ptr;
    use windows_sys::Win32::{
        Foundation::GetLastError,
        Security::Cryptography::{CRYPT_INTEGER_BLOB, CRYPTPROTECT_UI_FORBIDDEN, CryptProtectData},
    };

    let input = CRYPT_INTEGER_BLOB {
        cbData: u32::try_from(plaintext.len()).map_err(|_| key_unavailable())?,
        pbData: plaintext.as_ptr().cast_mut(),
    };
    let entropy = CRYPT_INTEGER_BLOB {
        cbData: DPAPI_ENTROPY.len() as u32,
        pbData: DPAPI_ENTROPY.as_ptr().cast_mut(),
    };
    let mut output = CRYPT_INTEGER_BLOB {
        cbData: 0,
        pbData: ptr::null_mut(),
    };
    let ok = unsafe {
        CryptProtectData(
            &input,
            ptr::null(),
            &entropy,
            ptr::null(),
            ptr::null(),
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut output,
        )
    };
    if ok == 0 {
        let _code = unsafe { GetLastError() };
        return Err(key_unavailable());
    }
    copy_and_free_dpapi_output(output)
}

#[cfg(windows)]
fn dpapi_unprotect(protected: &[u8]) -> Result<Vec<u8>, FoundationError> {
    use std::ptr;
    use windows_sys::Win32::{
        Foundation::GetLastError,
        Security::Cryptography::{
            CRYPT_INTEGER_BLOB, CRYPTPROTECT_UI_FORBIDDEN, CryptUnprotectData,
        },
    };

    let input = CRYPT_INTEGER_BLOB {
        cbData: u32::try_from(protected.len()).map_err(|_| key_unavailable())?,
        pbData: protected.as_ptr().cast_mut(),
    };
    let entropy = CRYPT_INTEGER_BLOB {
        cbData: DPAPI_ENTROPY.len() as u32,
        pbData: DPAPI_ENTROPY.as_ptr().cast_mut(),
    };
    let mut output = CRYPT_INTEGER_BLOB {
        cbData: 0,
        pbData: ptr::null_mut(),
    };
    let ok = unsafe {
        CryptUnprotectData(
            &input,
            ptr::null_mut(),
            &entropy,
            ptr::null(),
            ptr::null(),
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut output,
        )
    };
    if ok == 0 {
        let _code = unsafe { GetLastError() };
        return Err(key_unavailable());
    }
    copy_and_free_dpapi_output(output)
}

#[cfg(windows)]
fn copy_and_free_dpapi_output(
    output: windows_sys::Win32::Security::Cryptography::CRYPT_INTEGER_BLOB,
) -> Result<Vec<u8>, FoundationError> {
    use std::{ffi::c_void, slice};
    use windows_sys::Win32::Foundation::LocalFree;

    if output.pbData.is_null() || output.cbData == 0 {
        if !output.pbData.is_null() {
            unsafe { LocalFree(output.pbData.cast::<c_void>()) };
        }
        return Err(key_unavailable());
    }
    let bytes = unsafe { slice::from_raw_parts(output.pbData, output.cbData as usize) };
    let copied = bytes.to_vec();
    for offset in 0..output.cbData as usize {
        unsafe { std::ptr::write_volatile(output.pbData.add(offset), 0) };
    }
    unsafe { LocalFree(output.pbData.cast::<c_void>()) };
    Ok(copied)
}

#[cfg(not(windows))]
fn dpapi_protect(_plaintext: &[u8]) -> Result<Vec<u8>, FoundationError> {
    Err(key_unavailable())
}

#[cfg(not(windows))]
fn dpapi_unprotect(_protected: &[u8]) -> Result<Vec<u8>, FoundationError> {
    Err(key_unavailable())
}

fn map_keyring_io(error: io::Error) -> FoundationError {
    if error.raw_os_error() == Some(112) {
        FoundationError::new(ErrorCode::DiskFull, "The disk is full.")
    } else {
        data_root_unavailable()
    }
}

fn data_root_unavailable() -> FoundationError {
    FoundationError::new(
        ErrorCode::DataRootUnavailable,
        "The local application data directory is unavailable.",
    )
}

fn key_unavailable() -> FoundationError {
    FoundationError::key_unavailable()
}

fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(DIGITS[(byte >> 4) as usize] as char);
        output.push(DIGITS[(byte & 0x0f) as usize] as char);
    }
    output
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    let mut difference = 0_u8;
    for (left, right) in left.iter().zip(right) {
        difference |= left ^ right;
    }
    difference == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exclusive_copy_fallback_handles_unavailable_atomic_link() {
        let root = std::env::temp_dir().join(format!(
            "desktop-notes-keyring-commit-{}",
            std::process::id()
        ));
        fs::create_dir_all(&root).unwrap();
        let temporary = root.join("keyring.tmp");
        let destination = root.join("keyring.json");
        let encoded = b"synthetic-wrapped-keyring";
        fs::write(&temporary, encoded).unwrap();

        commit_keyring_document_with(&temporary, &destination, encoded, |_, _| {
            Err(io::Error::from(io::ErrorKind::CrossesDevices))
        })
        .unwrap();

        assert_eq!(fs::read(&destination).unwrap(), encoded);
        assert!(!temporary.exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn failed_fallback_write_removes_exclusively_created_destination() {
        let root = std::env::temp_dir().join(format!(
            "desktop-notes-keyring-partial-{}",
            std::process::id()
        ));
        fs::create_dir_all(&root).unwrap();
        let destination = root.join("keyring.json");

        let error =
            persist_keyring_document_with(&destination, b"complete-keyring", |file, bytes| {
                file.write_all(&bytes[..4])?;
                Err(io::Error::new(io::ErrorKind::WriteZero, "injected"))
            })
            .unwrap_err();

        assert_eq!(error.code(), ErrorCode::DataRootUnavailable);
        assert!(!destination.exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn fallback_never_replaces_or_removes_an_existing_destination() {
        let root = std::env::temp_dir().join(format!(
            "desktop-notes-keyring-existing-{}",
            std::process::id()
        ));
        fs::create_dir_all(&root).unwrap();
        let temporary = root.join("keyring.tmp");
        let destination = root.join("keyring.json");
        fs::write(&temporary, b"new-keyring").unwrap();
        fs::write(&destination, b"existing-keyring").unwrap();

        let error =
            commit_keyring_document_with(&temporary, &destination, b"new-keyring", |_, _| {
                Err(io::Error::from(io::ErrorKind::CrossesDevices))
            })
            .unwrap_err();

        assert_eq!(error.code(), ErrorCode::KeyUnavailable);
        assert_eq!(fs::read(&destination).unwrap(), b"existing-keyring");
        assert!(temporary.exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn failed_fallback_sync_removes_exclusively_created_destination() {
        let root =
            std::env::temp_dir().join(format!("desktop-notes-keyring-sync-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        let destination = root.join("keyring.json");
        let mut exclusive = ExclusiveDestination::create(&destination).unwrap();
        exclusive.file_mut().write_all(b"complete-keyring").unwrap();

        let error = exclusive
            .sync_close_and_verify_with(
                b"complete-keyring",
                |_| Err(io::Error::other("injected sync failure")),
                |path| fs::read(path),
            )
            .unwrap_err();

        assert_eq!(error.code(), ErrorCode::DataRootUnavailable);
        assert!(!destination.exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn failed_fallback_readback_removes_exclusively_created_destination() {
        let root = std::env::temp_dir().join(format!(
            "desktop-notes-keyring-readback-{}",
            std::process::id()
        ));
        fs::create_dir_all(&root).unwrap();
        let destination = root.join("keyring.json");
        let mut exclusive = ExclusiveDestination::create(&destination).unwrap();
        exclusive.file_mut().write_all(b"complete-keyring").unwrap();

        let error = exclusive
            .sync_close_and_verify_with(
                b"complete-keyring",
                |file| file.sync_all(),
                |_| Err(io::Error::other("injected readback failure")),
            )
            .unwrap_err();

        assert_eq!(error.code(), ErrorCode::DataRootUnavailable);
        assert!(!destination.exists());
        fs::remove_dir_all(root).unwrap();
    }
}
