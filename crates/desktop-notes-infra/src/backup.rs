use std::{
    collections::HashSet,
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    path::{Path, PathBuf},
    sync::{Mutex, TryLockError},
};

use aes_gcm::{
    Aes256Gcm, Nonce,
    aead::{Aead, KeyInit, Payload},
};
use desktop_notes_core::{ErrorCode, FoundationError, SecretKey, validate_note_id};
use hkdf::Hkdf;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

use crate::{
    EncryptedAssetFiles, SqlCipherStore,
    assets::AssetBackupReadGuard,
    sqlcipher::{BackupSnapshotAsset, BackupSnapshotMetadata},
};

const DNBAK_MAGIC: &[u8; 8] = b"DNBAKv1\0";
const DNBAK_END_MAGIC: &[u8; 8] = b"DNBEND1\0";
const DNBAK_ALGORITHM_AES_256_GCM: u8 = 1;
const DNBAK_PROFILE_DPAPI_CURRENT_USER: u8 = 1;
const DNBAK_SALT_LEN: usize = 16;
const DNBAK_NONCE_PREFIX_LEN: usize = 8;
const DNBAK_TAG_LEN: usize = 16;
const DNBAK_CONTAINER_KEY_INFO: &[u8] = b"dnbak-container-v1";
const BACKUP_ID_LEN: usize = 36;
const LOCAL_DAY_LEN: usize = 10;
const MAX_WRAPPED_ROOT_LEN: usize = 64 * 1024;
const MAX_MANIFEST_LEN: usize = 4 * 1024 * 1024;
const MAX_COMPONENTS: u32 = 100_000;
const MAX_COMPONENT_PATH_LEN: usize = 256;
const MANIFEST_PATH: &str = "manifest.json";
const DATABASE_PATH: &str = "data/desktop-notes.db";
const INVALID_GRACE_MS: i64 = 24 * 60 * 60 * 1000;

pub const DNBAK_FORMAT_VERSION: u16 = 1;
pub const DNBAK_CHUNK_SIZE: u32 = 64 * 1024;
pub const DEFAULT_AUTOMATIC_RETENTION: usize = 30;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BackupClassification {
    Valid,
    Incomplete,
    Corrupt,
    Unsupported,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BackupTrigger {
    Startup,
    Resumed,
    Interval,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BackupHealthState {
    Never,
    Healthy,
    Failed,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupHealth {
    pub state: BackupHealthState,
    pub last_success_at_ms: Option<i64>,
    pub last_success_local_day: Option<String>,
    pub valid_generation_count: usize,
    pub last_error_code: Option<ErrorCode>,
}

impl Default for BackupHealth {
    fn default() -> Self {
        Self {
            state: BackupHealthState::Never,
            last_success_at_ms: None,
            last_success_local_day: None,
            valid_generation_count: 0,
            last_error_code: None,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ValidatedBackup {
    pub backup_id: String,
    pub local_day: String,
    pub created_at_ms: i64,
    pub source_schema_version: u32,
    pub asset_count: usize,
    pub package_size: u64,
    pub package_sha256_hex: String,
    pub storage_path: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BackupDiscovery {
    pub path: PathBuf,
    pub classification: BackupClassification,
    pub validated: Option<ValidatedBackup>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BackupRunOutcome {
    Created(ValidatedBackup),
    SkippedSameDay(ValidatedBackup),
}

pub struct AutomaticBackupManager {
    data_root: PathBuf,
    backup_key: SecretKey,
    wrapped_root: Zeroizing<Vec<u8>>,
    writer: Mutex<()>,
    health: Mutex<BackupHealth>,
    retention: usize,
}

impl AutomaticBackupManager {
    pub fn new(
        data_root: PathBuf,
        backup_key: SecretKey,
        wrapped_root: Vec<u8>,
    ) -> Result<Self, FoundationError> {
        if wrapped_root.is_empty() || wrapped_root.len() > MAX_WRAPPED_ROOT_LEN {
            return Err(FoundationError::key_unavailable());
        }
        Ok(Self {
            data_root,
            backup_key,
            wrapped_root: Zeroizing::new(wrapped_root),
            writer: Mutex::new(()),
            health: Mutex::new(BackupHealth::default()),
            retention: DEFAULT_AUTOMATIC_RETENTION,
        })
    }

    pub fn backup_directory(&self) -> PathBuf {
        self.data_root.join("backups").join("automatic")
    }

    pub fn health(&self) -> BackupHealth {
        self.health
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    pub fn discover(&self) -> Result<Vec<BackupDiscovery>, FoundationError> {
        discover_directory(&self.backup_directory(), &self.backup_key)
    }

    pub fn refresh_health(&self) -> Result<BackupHealth, FoundationError> {
        let discoveries = self.discover()?;
        let health = health_from_discoveries(&discoveries, None);
        *self
            .health
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = health.clone();
        Ok(health)
    }

    pub fn run_if_due(
        &self,
        database: &SqlCipherStore,
        database_key: &SecretKey,
        asset_files: &EncryptedAssetFiles,
        created_at_ms: i64,
        local_day: &str,
        _trigger: BackupTrigger,
    ) -> Result<BackupRunOutcome, FoundationError> {
        validate_local_day(local_day).map_err(|_| FoundationError::backup_failed())?;
        if created_at_ms < 0 {
            return Err(FoundationError::backup_failed());
        }
        let _writer = match self.writer.try_lock() {
            Ok(writer) => writer,
            Err(TryLockError::WouldBlock) => return Err(FoundationError::backup_busy()),
            Err(TryLockError::Poisoned(_)) => return Err(FoundationError::backup_failed()),
        };

        let before = match self.discover() {
            Ok(discoveries) => discoveries,
            Err(error) => {
                let mut health = self
                    .health
                    .lock()
                    .map_err(|_| FoundationError::backup_failed())?;
                health.state = BackupHealthState::Failed;
                health.last_error_code = Some(error.code());
                return Err(error);
            }
        };
        if let Some(existing) = newest_valid(&before)
            && existing.local_day == local_day
        {
            let valid = valid_backups(&before);
            let _ = database.cache_valid_backups(&valid);
            let health = health_from_discoveries(&before, None);
            *self
                .health
                .lock()
                .map_err(|_| FoundationError::backup_failed())? = health;
            return Ok(BackupRunOutcome::SkippedSameDay(existing));
        }

        let result = self.create_backup_locked(
            database,
            database_key,
            asset_files,
            created_at_ms,
            local_day,
        );
        match result {
            Ok(created) => {
                if let Err(error) = self.apply_retention(&created.backup_id) {
                    let discoveries = self.discover().unwrap_or(before);
                    *self
                        .health
                        .lock()
                        .map_err(|_| FoundationError::backup_failed())? =
                        health_from_discoveries(&discoveries, Some(error.code()));
                    return Err(error);
                }
                let after = match self.discover() {
                    Ok(discoveries) => discoveries,
                    Err(error) => {
                        let mut health = self
                            .health
                            .lock()
                            .map_err(|_| FoundationError::backup_failed())?;
                        health.state = BackupHealthState::Failed;
                        health.last_error_code = Some(error.code());
                        return Err(error);
                    }
                };
                let valid = valid_backups(&after);
                let _ = database.cache_valid_backups(&valid);
                *self
                    .health
                    .lock()
                    .map_err(|_| FoundationError::backup_failed())? =
                    health_from_discoveries(&after, None);
                Ok(BackupRunOutcome::Created(created))
            }
            Err(error) => {
                let discoveries = self.discover().unwrap_or(before);
                *self
                    .health
                    .lock()
                    .map_err(|_| FoundationError::backup_failed())? =
                    health_from_discoveries(&discoveries, Some(error.code()));
                Err(error)
            }
        }
    }

    pub fn validate(&self, path: &Path) -> Result<ValidatedBackup, FoundationError> {
        validate_package(path, &self.backup_key).map_err(ValidationFailure::into_foundation)
    }

    pub fn extract_validated_to_isolated(
        &self,
        package: &Path,
        destination_root: &Path,
    ) -> Result<ValidatedBackup, FoundationError> {
        let validated = self.validate(package)?;
        if destination_root.exists() {
            return Err(FoundationError::backup_failed());
        }
        fs::create_dir_all(destination_root).map_err(map_backup_io)?;
        if let Err(error) = extract_package(package, &self.backup_key, destination_root) {
            let _ = fs::remove_dir_all(destination_root);
            return Err(error.into_foundation());
        }
        Ok(validated)
    }

    fn create_backup_locked(
        &self,
        database: &SqlCipherStore,
        database_key: &SecretKey,
        asset_files: &EncryptedAssetFiles,
        created_at_ms: i64,
        local_day: &str,
    ) -> Result<ValidatedBackup, FoundationError> {
        let backup_id = new_backup_id()?;
        let filename = backup_filename(local_day, &backup_id)?;
        let backup_directory = self.backup_directory();
        fs::create_dir_all(&backup_directory).map_err(map_backup_io)?;
        let package_path = backup_directory.join(filename);
        let staging_root = self.data_root.join("staging").join("backup");
        fs::create_dir_all(&staging_root).map_err(map_backup_io)?;
        let snapshot_path = staging_root.join(format!("{backup_id}.dbsnap"));
        let snapshot_cleanup = SnapshotCleanup(snapshot_path.clone());

        let asset_guard = asset_files.backup_read_guard()?;
        let snapshot = database.create_backup_snapshot(&snapshot_path, database_key)?;
        let prepared = prepare_package(
            &snapshot_path,
            snapshot,
            &asset_guard,
            &backup_id,
            created_at_ms,
            local_day,
        )?;
        write_package(
            &package_path,
            &self.backup_key,
            self.wrapped_root.as_slice(),
            &prepared,
        )?;
        drop(snapshot_cleanup);

        let validated = validate_package(&package_path, &self.backup_key)
            .map_err(ValidationFailure::into_foundation)?;
        if validated.backup_id != backup_id {
            return Err(FoundationError::backup_corrupted());
        }
        Ok(validated)
    }

    fn apply_retention(&self, protected_backup_id: &str) -> Result<(), FoundationError> {
        let mut valid = self
            .discover()?
            .into_iter()
            .filter_map(|entry| entry.validated.map(|validated| (entry.path, validated)))
            .collect::<Vec<_>>();
        valid.sort_by(|left, right| {
            right
                .1
                .created_at_ms
                .cmp(&left.1.created_at_ms)
                .then_with(|| right.1.backup_id.cmp(&left.1.backup_id))
        });
        for (path, backup) in valid.into_iter().skip(self.retention) {
            if backup.backup_id == protected_backup_id {
                continue;
            }
            fs::remove_file(path).map_err(map_backup_io)?;
        }
        Ok(())
    }

    pub fn cleanup_invalid(&self, now_ms: i64) -> Result<u32, FoundationError> {
        let _writer = self
            .writer
            .try_lock()
            .map_err(|_| FoundationError::backup_busy())?;
        let mut removed = 0_u32;
        for discovery in self.discover()? {
            if discovery.classification == BackupClassification::Valid {
                continue;
            }
            let Some(filename) = discovery.path.file_name().and_then(|value| value.to_str()) else {
                continue;
            };
            if parse_backup_filename(filename).is_none() {
                continue;
            }
            let metadata = match fs::metadata(&discovery.path) {
                Ok(metadata) => metadata,
                Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
                Err(error) => return Err(map_backup_io(error)),
            };
            let modified_ms = metadata
                .modified()
                .ok()
                .and_then(|modified| modified.duration_since(std::time::UNIX_EPOCH).ok())
                .and_then(|duration| i64::try_from(duration.as_millis()).ok());
            if modified_ms.is_none_or(|modified| now_ms.saturating_sub(modified) < INVALID_GRACE_MS)
            {
                continue;
            }
            fs::remove_file(&discovery.path).map_err(map_backup_io)?;
            removed = removed.saturating_add(1);
        }
        Ok(removed)
    }
}

struct SnapshotCleanup(PathBuf);

impl Drop for SnapshotCleanup {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
        let _ = fs::remove_file(self.0.with_extension("dbsnap-wal"));
        let _ = fs::remove_file(self.0.with_extension("dbsnap-shm"));
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct BackupManifest {
    package_version: u16,
    backup_id: String,
    created_at_ms: i64,
    local_day: String,
    source_schema_version: u32,
    body_schema_version: u32,
    database: ManifestItem,
    assets: Vec<ManifestAsset>,
    authoritative_data: Vec<String>,
    rebuildable_derived_data: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct ManifestItem {
    path: String,
    size: u64,
    sha256: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct ManifestAsset {
    asset_id: String,
    path: String,
    size: u64,
    sha256_cipher: String,
}

struct PreparedPackage {
    header: PackageHeader,
    manifest: Vec<u8>,
    database: PreparedSource,
    assets: Vec<PreparedAsset>,
}

struct PreparedSource {
    path: PathBuf,
    size: u64,
    sha256: [u8; 32],
}

struct PreparedAsset {
    asset_id: String,
    storage_relpath: String,
    source: PreparedSource,
}

#[derive(Clone)]
struct PackageHeader {
    backup_id: String,
    local_day: String,
    created_at_ms: i64,
    source_schema_version: u32,
    payload_length: u64,
    component_count: u32,
    salt: [u8; DNBAK_SALT_LEN],
    nonce_prefix: [u8; DNBAK_NONCE_PREFIX_LEN],
    wrapped_root: Vec<u8>,
    encoded: Vec<u8>,
}

fn prepare_package(
    snapshot_path: &Path,
    snapshot: BackupSnapshotMetadata,
    assets: &AssetBackupReadGuard<'_>,
    backup_id: &str,
    created_at_ms: i64,
    local_day: &str,
) -> Result<PreparedPackage, FoundationError> {
    let database = source_digest(snapshot_path)?;
    let mut prepared_assets = Vec::with_capacity(snapshot.assets.len());
    let mut manifest_assets = Vec::with_capacity(snapshot.assets.len());
    for asset in snapshot.assets {
        let prepared = prepare_asset_source(assets, asset)?;
        manifest_assets.push(ManifestAsset {
            asset_id: prepared.asset_id.clone(),
            path: prepared.storage_relpath.clone(),
            size: prepared.source.size,
            sha256_cipher: hex(&prepared.source.sha256),
        });
        prepared_assets.push(prepared);
    }
    let manifest = BackupManifest {
        package_version: DNBAK_FORMAT_VERSION,
        backup_id: backup_id.to_owned(),
        created_at_ms,
        local_day: local_day.to_owned(),
        source_schema_version: snapshot.schema_version,
        body_schema_version: 1,
        database: ManifestItem {
            path: DATABASE_PATH.to_owned(),
            size: database.size,
            sha256: hex(&database.sha256),
        },
        assets: manifest_assets,
        authoritative_data: vec![
            "notes".into(),
            "note_dates".into(),
            "rich_text_json".into(),
            "tags".into(),
            "pin".into(),
            "encrypted_images".into(),
            "quick_capture_notes".into(),
            "schema_and_settings".into(),
        ],
        rebuildable_derived_data: vec!["fts5_search_index".into()],
    };
    let manifest = serde_json::to_vec(&manifest).map_err(|_| FoundationError::backup_failed())?;
    if manifest.is_empty() || manifest.len() > MAX_MANIFEST_LEN {
        return Err(FoundationError::backup_failed());
    }
    let component_count = u32::try_from(2_usize.saturating_add(prepared_assets.len()))
        .map_err(|_| FoundationError::backup_failed())?;
    if component_count > MAX_COMPONENTS {
        return Err(FoundationError::backup_failed());
    }
    let mut payload_length = framed_length(MANIFEST_PATH, manifest.len() as u64)?
        .checked_add(framed_length(DATABASE_PATH, database.size)?)
        .ok_or_else(FoundationError::backup_failed)?;
    for asset in &prepared_assets {
        payload_length = payload_length
            .checked_add(framed_length(&asset.storage_relpath, asset.source.size)?)
            .ok_or_else(FoundationError::backup_failed)?;
    }
    Ok(PreparedPackage {
        header: PackageHeader {
            backup_id: backup_id.to_owned(),
            local_day: local_day.to_owned(),
            created_at_ms,
            source_schema_version: snapshot.schema_version,
            payload_length,
            component_count,
            salt: random_array()?,
            nonce_prefix: random_array()?,
            wrapped_root: Vec::new(),
            encoded: Vec::new(),
        },
        manifest,
        database,
        assets: prepared_assets,
    })
}

fn prepare_asset_source(
    guard: &AssetBackupReadGuard<'_>,
    asset: BackupSnapshotAsset,
) -> Result<PreparedAsset, FoundationError> {
    let source_path = guard.source_path(
        &asset.asset_id,
        &asset.storage_relpath,
        asset.byte_size_cipher,
    )?;
    let mut file = guard.open_ciphertext(
        &asset.asset_id,
        &asset.storage_relpath,
        asset.byte_size_cipher,
    )?;
    let sha256 = digest_reader(&mut file)?;
    Ok(PreparedAsset {
        asset_id: asset.asset_id,
        storage_relpath: asset.storage_relpath,
        source: PreparedSource {
            path: source_path,
            size: asset.byte_size_cipher,
            sha256,
        },
    })
}

fn source_digest(path: &Path) -> Result<PreparedSource, FoundationError> {
    let mut file = File::open(path).map_err(map_backup_io)?;
    let size = file.metadata().map_err(map_backup_io)?.len();
    if size == 0 {
        return Err(FoundationError::backup_failed());
    }
    let sha256 = digest_reader(&mut file)?;
    Ok(PreparedSource {
        path: path.to_owned(),
        size,
        sha256,
    })
}

fn digest_reader(reader: &mut impl Read) -> Result<[u8; 32], FoundationError> {
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = reader.read(&mut buffer).map_err(map_backup_io)?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
    Ok(digest.finalize().into())
}

fn write_package(
    path: &Path,
    backup_key: &SecretKey,
    wrapped_root: &[u8],
    prepared: &PreparedPackage,
) -> Result<(), FoundationError> {
    let mut header = prepared.header.clone();
    header.wrapped_root = wrapped_root.to_vec();
    header.encoded = encode_header(&header)?;
    let header_hash: [u8; 32] = Sha256::digest(&header.encoded).into();
    let container_key = derive_container_key(backup_key, &header.backup_id, &header.salt)?;
    let cipher = Aes256Gcm::new_from_slice(container_key.as_slice())
        .map_err(|_| FoundationError::backup_failed())?;
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(map_backup_io)?;
    file.write_all(&header.encoded).map_err(map_backup_io)?;
    let mut writer = EncryptedPayloadWriter::new(
        &mut file,
        cipher,
        header_hash,
        header.backup_id.as_bytes(),
        header.nonce_prefix,
        header.payload_length,
    );

    writer.write_component(
        1,
        MANIFEST_PATH,
        &prepared.manifest[..],
        prepared.manifest.len() as u64,
        Sha256::digest(&prepared.manifest).into(),
    )?;
    writer.write_component_file(2, DATABASE_PATH, &prepared.database)?;
    for asset in &prepared.assets {
        writer.write_component_asset(3, &asset.storage_relpath, asset)?;
    }
    writer.finish()?;
    file.sync_all().map_err(map_backup_io)?;
    drop(file);
    Ok(())
}

struct EncryptedPayloadWriter<'a, W: Write> {
    output: &'a mut W,
    cipher: Aes256Gcm,
    header_hash: [u8; 32],
    backup_id: Vec<u8>,
    nonce_prefix: [u8; DNBAK_NONCE_PREFIX_LEN],
    expected_plain_length: u64,
    plain_length: u64,
    chunk_index: u32,
    buffer: Vec<u8>,
    digest: Sha256,
}

impl<'a, W: Write> EncryptedPayloadWriter<'a, W> {
    fn new(
        output: &'a mut W,
        cipher: Aes256Gcm,
        header_hash: [u8; 32],
        backup_id: &[u8],
        nonce_prefix: [u8; DNBAK_NONCE_PREFIX_LEN],
        expected_plain_length: u64,
    ) -> Self {
        Self {
            output,
            cipher,
            header_hash,
            backup_id: backup_id.to_vec(),
            nonce_prefix,
            expected_plain_length,
            plain_length: 0,
            chunk_index: 0,
            buffer: Vec::with_capacity(DNBAK_CHUNK_SIZE as usize),
            digest: Sha256::new(),
        }
    }

    fn write_component(
        &mut self,
        kind: u8,
        path: &str,
        mut input: impl Read,
        length: u64,
        sha256: [u8; 32],
    ) -> Result<(), FoundationError> {
        let path_bytes = path.as_bytes();
        let path_length =
            u16::try_from(path_bytes.len()).map_err(|_| FoundationError::backup_failed())?;
        self.write_plain(&[kind])?;
        self.write_plain(&path_length.to_le_bytes())?;
        self.write_plain(path_bytes)?;
        self.write_plain(&length.to_le_bytes())?;
        self.write_plain(&sha256)?;
        let mut remaining = length;
        let mut buffer = [0_u8; 64 * 1024];
        while remaining > 0 {
            let wanted =
                usize::try_from(remaining.min(buffer.len() as u64)).unwrap_or(buffer.len());
            let read = input.read(&mut buffer[..wanted]).map_err(map_backup_io)?;
            if read == 0 {
                return Err(FoundationError::backup_corrupted());
            }
            self.write_plain(&buffer[..read])?;
            remaining -= read as u64;
        }
        let mut extra = [0_u8; 1];
        if input.read(&mut extra).map_err(map_backup_io)? != 0 {
            return Err(FoundationError::backup_corrupted());
        }
        Ok(())
    }

    fn write_component_file(
        &mut self,
        kind: u8,
        path: &str,
        source: &PreparedSource,
    ) -> Result<(), FoundationError> {
        let file = File::open(&source.path).map_err(map_backup_io)?;
        self.write_component(kind, path, file, source.size, source.sha256)
    }

    fn write_component_asset(
        &mut self,
        kind: u8,
        path: &str,
        asset: &PreparedAsset,
    ) -> Result<(), FoundationError> {
        let file = File::open(&asset.source.path).map_err(map_backup_io)?;
        self.write_component(kind, path, file, asset.source.size, asset.source.sha256)
    }

    fn write_plain(&mut self, mut bytes: &[u8]) -> Result<(), FoundationError> {
        while !bytes.is_empty() {
            let remaining = DNBAK_CHUNK_SIZE as usize - self.buffer.len();
            let take = remaining.min(bytes.len());
            self.buffer.extend_from_slice(&bytes[..take]);
            bytes = &bytes[take..];
            if self.buffer.len() == DNBAK_CHUNK_SIZE as usize {
                self.flush_chunk()?;
            }
        }
        Ok(())
    }

    fn flush_chunk(&mut self) -> Result<(), FoundationError> {
        if self.buffer.is_empty() {
            return Ok(());
        }
        let plain = std::mem::take(&mut self.buffer);
        self.digest.update(&plain);
        self.plain_length = self
            .plain_length
            .checked_add(plain.len() as u64)
            .ok_or_else(FoundationError::backup_failed)?;
        let nonce = chunk_nonce(&self.nonce_prefix, self.chunk_index);
        let aad = chunk_aad(
            &self.header_hash,
            &self.backup_id,
            self.chunk_index,
            plain.len() as u32,
            self.expected_plain_length,
        );
        let encrypted = self
            .cipher
            .encrypt(
                &nonce,
                Payload {
                    msg: &plain,
                    aad: &aad,
                },
            )
            .map_err(|_| FoundationError::backup_failed())?;
        self.output
            .write_all(&(encrypted.len() as u32).to_le_bytes())
            .and_then(|_| self.output.write_all(&encrypted))
            .map_err(map_backup_io)?;
        self.chunk_index = self
            .chunk_index
            .checked_add(1)
            .ok_or_else(FoundationError::backup_failed)?;
        self.buffer = Vec::with_capacity(DNBAK_CHUNK_SIZE as usize);
        Ok(())
    }

    fn finish(mut self) -> Result<(), FoundationError> {
        self.flush_chunk()?;
        if self.plain_length != self.expected_plain_length || self.chunk_index == 0 {
            return Err(FoundationError::backup_corrupted());
        }
        let payload_digest: [u8; 32] = self.digest.finalize().into();
        let footer_aad = footer_aad(
            &self.header_hash,
            &self.backup_id,
            self.chunk_index,
            self.plain_length,
            &payload_digest,
        );
        let tag = self
            .cipher
            .encrypt(
                &chunk_nonce(&self.nonce_prefix, u32::MAX),
                Payload {
                    msg: &[],
                    aad: &footer_aad,
                },
            )
            .map_err(|_| FoundationError::backup_failed())?;
        if tag.len() != DNBAK_TAG_LEN {
            return Err(FoundationError::backup_failed());
        }
        self.output
            .write_all(DNBAK_END_MAGIC)
            .map_err(map_backup_io)?;
        self.output
            .write_all(&self.chunk_index.to_le_bytes())
            .map_err(map_backup_io)?;
        self.output
            .write_all(&self.plain_length.to_le_bytes())
            .map_err(map_backup_io)?;
        self.output
            .write_all(&payload_digest)
            .map_err(map_backup_io)?;
        self.output.write_all(&tag).map_err(map_backup_io)?;
        Ok(())
    }
}

#[derive(Debug)]
enum ValidationFailure {
    Incomplete,
    Corrupt,
    Unsupported,
    Io(io::Error),
}

impl ValidationFailure {
    fn into_foundation(self) -> FoundationError {
        match self {
            Self::Unsupported => FoundationError::backup_unsupported(),
            Self::Incomplete | Self::Corrupt => FoundationError::backup_corrupted(),
            Self::Io(error) => map_backup_io(error),
        }
    }

    fn classification(&self) -> BackupClassification {
        match self {
            Self::Incomplete => BackupClassification::Incomplete,
            Self::Unsupported => BackupClassification::Unsupported,
            Self::Corrupt | Self::Io(_) => BackupClassification::Corrupt,
        }
    }
}

impl From<io::Error> for ValidationFailure {
    fn from(error: io::Error) -> Self {
        if error.kind() == io::ErrorKind::UnexpectedEof {
            Self::Incomplete
        } else {
            Self::Io(error)
        }
    }
}

fn validate_package(
    path: &Path,
    backup_key: &SecretKey,
) -> Result<ValidatedBackup, ValidationFailure> {
    validate_or_extract(path, backup_key, None)
}

fn extract_package(
    path: &Path,
    backup_key: &SecretKey,
    destination_root: &Path,
) -> Result<ValidatedBackup, ValidationFailure> {
    validate_or_extract(path, backup_key, Some(destination_root))
}

fn validate_or_extract(
    path: &Path,
    backup_key: &SecretKey,
    destination_root: Option<&Path>,
) -> Result<ValidatedBackup, ValidationFailure> {
    let filename = path
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or(ValidationFailure::Corrupt)?;
    let (filename_day, filename_id) =
        parse_backup_filename(filename).ok_or(ValidationFailure::Corrupt)?;
    let mut file = File::open(path)?;
    let header = decode_header(&mut file)?;
    if header.backup_id != filename_id || header.local_day != filename_day {
        return Err(ValidationFailure::Corrupt);
    }
    let header_hash: [u8; 32] = Sha256::digest(&header.encoded).into();
    let container_key = derive_container_key(backup_key, &header.backup_id, &header.salt)
        .map_err(|_| ValidationFailure::Corrupt)?;
    let cipher = Aes256Gcm::new_from_slice(container_key.as_slice())
        .map_err(|_| ValidationFailure::Corrupt)?;
    let mut reader = DecryptedPayloadReader::new(&mut file, cipher, &header, header_hash);
    let mut observed = Vec::new();
    let mut manifest_bytes = Vec::new();
    let mut paths = HashSet::new();
    for index in 0..header.component_count {
        let kind = read_u8(&mut reader)?;
        let path_length = usize::from(read_u16(&mut reader)?);
        if path_length == 0 || path_length > MAX_COMPONENT_PATH_LEN {
            return Err(ValidationFailure::Corrupt);
        }
        let mut path_bytes = vec![0_u8; path_length];
        reader.read_exact(&mut path_bytes)?;
        let component_path =
            String::from_utf8(path_bytes).map_err(|_| ValidationFailure::Corrupt)?;
        validate_component_path(kind, index, &component_path)?;
        if !paths.insert(component_path.clone()) {
            return Err(ValidationFailure::Corrupt);
        }
        let length = read_u64(&mut reader)?;
        let mut expected_sha = [0_u8; 32];
        reader.read_exact(&mut expected_sha)?;
        if kind == 1 && length as usize > MAX_MANIFEST_LEN {
            return Err(ValidationFailure::Corrupt);
        }
        let mut output = if let Some(root) = destination_root {
            if kind == 1 {
                None
            } else {
                Some(create_extraction_file(root, &component_path)?)
            }
        } else {
            None
        };
        let mut digest = Sha256::new();
        let mut remaining = length;
        let mut buffer = [0_u8; 64 * 1024];
        while remaining > 0 {
            let wanted =
                usize::try_from(remaining.min(buffer.len() as u64)).unwrap_or(buffer.len());
            reader.read_exact(&mut buffer[..wanted])?;
            digest.update(&buffer[..wanted]);
            if kind == 1 {
                manifest_bytes.extend_from_slice(&buffer[..wanted]);
            }
            if let Some(file) = output.as_mut() {
                file.write_all(&buffer[..wanted])?;
            }
            remaining -= wanted as u64;
        }
        if let Some(file) = output {
            file.sync_all()?;
        }
        let actual_sha: [u8; 32] = digest.finalize().into();
        if actual_sha != expected_sha {
            return Err(ValidationFailure::Corrupt);
        }
        observed.push((kind, component_path, length, actual_sha));
    }
    reader.finish()?;
    let manifest: BackupManifest =
        serde_json::from_slice(&manifest_bytes).map_err(|_| ValidationFailure::Corrupt)?;
    validate_manifest(&header, &manifest, &observed)?;
    let package_size = fs::metadata(path)?.len();
    let mut package = File::open(path)?;
    let package_sha256 = digest_reader_validation(&mut package)?;
    Ok(ValidatedBackup {
        backup_id: header.backup_id,
        local_day: header.local_day,
        created_at_ms: header.created_at_ms,
        source_schema_version: header.source_schema_version,
        asset_count: manifest.assets.len(),
        package_size,
        package_sha256_hex: hex(&package_sha256),
        storage_path: filename.to_owned(),
    })
}

struct DecryptedPayloadReader<'a> {
    input: &'a mut File,
    cipher: Aes256Gcm,
    header_hash: [u8; 32],
    backup_id: Vec<u8>,
    nonce_prefix: [u8; DNBAK_NONCE_PREFIX_LEN],
    expected_plain_length: u64,
    remaining_plain: u64,
    next_chunk: u32,
    current: Zeroizing<Vec<u8>>,
    current_offset: usize,
    digest: Sha256,
}

impl<'a> DecryptedPayloadReader<'a> {
    fn new(
        input: &'a mut File,
        cipher: Aes256Gcm,
        header: &PackageHeader,
        header_hash: [u8; 32],
    ) -> Self {
        Self {
            input,
            cipher,
            header_hash,
            backup_id: header.backup_id.as_bytes().to_vec(),
            nonce_prefix: header.nonce_prefix,
            expected_plain_length: header.payload_length,
            remaining_plain: header.payload_length,
            next_chunk: 0,
            current: Zeroizing::new(Vec::new()),
            current_offset: 0,
            digest: Sha256::new(),
        }
    }

    fn load_chunk(&mut self) -> Result<(), ValidationFailure> {
        if self.remaining_plain == 0 {
            return Ok(());
        }
        let expected_plain = self.remaining_plain.min(DNBAK_CHUNK_SIZE as u64) as usize;
        let encrypted_length = read_u32(self.input)? as usize;
        if encrypted_length != expected_plain + DNBAK_TAG_LEN {
            return Err(ValidationFailure::Corrupt);
        }
        let mut encrypted = vec![0_u8; encrypted_length];
        self.input.read_exact(&mut encrypted)?;
        let aad = chunk_aad(
            &self.header_hash,
            &self.backup_id,
            self.next_chunk,
            expected_plain as u32,
            self.expected_plain_length,
        );
        let plain = self
            .cipher
            .decrypt(
                &chunk_nonce(&self.nonce_prefix, self.next_chunk),
                Payload {
                    msg: &encrypted,
                    aad: &aad,
                },
            )
            .map_err(|_| ValidationFailure::Corrupt)?;
        if plain.len() != expected_plain {
            return Err(ValidationFailure::Corrupt);
        }
        self.digest.update(&plain);
        self.remaining_plain -= plain.len() as u64;
        self.next_chunk = self
            .next_chunk
            .checked_add(1)
            .ok_or(ValidationFailure::Corrupt)?;
        self.current = Zeroizing::new(plain);
        self.current_offset = 0;
        Ok(())
    }

    fn finish(self) -> Result<(), ValidationFailure> {
        if self.remaining_plain != 0 || self.current_offset != self.current.len() {
            return Err(ValidationFailure::Corrupt);
        }
        let mut magic = [0_u8; 8];
        self.input.read_exact(&mut magic)?;
        if &magic != DNBAK_END_MAGIC {
            return Err(ValidationFailure::Incomplete);
        }
        let chunk_count = read_u32(self.input)?;
        let plain_length = read_u64(self.input)?;
        let mut expected_digest = [0_u8; 32];
        self.input.read_exact(&mut expected_digest)?;
        let mut tag = [0_u8; DNBAK_TAG_LEN];
        self.input.read_exact(&mut tag)?;
        if chunk_count != self.next_chunk || plain_length != self.expected_plain_length {
            return Err(ValidationFailure::Corrupt);
        }
        let actual_digest: [u8; 32] = self.digest.finalize().into();
        if actual_digest != expected_digest {
            return Err(ValidationFailure::Corrupt);
        }
        let aad = footer_aad(
            &self.header_hash,
            &self.backup_id,
            chunk_count,
            plain_length,
            &expected_digest,
        );
        let opened = self
            .cipher
            .decrypt(
                &chunk_nonce(&self.nonce_prefix, u32::MAX),
                Payload {
                    msg: &tag,
                    aad: &aad,
                },
            )
            .map_err(|_| ValidationFailure::Corrupt)?;
        if !opened.is_empty() {
            return Err(ValidationFailure::Corrupt);
        }
        let mut extra = [0_u8; 1];
        if self.input.read(&mut extra)? != 0 {
            return Err(ValidationFailure::Corrupt);
        }
        Ok(())
    }
}

impl Read for DecryptedPayloadReader<'_> {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        if output.is_empty() {
            return Ok(0);
        }
        if self.current_offset == self.current.len() {
            self.load_chunk().map_err(validation_io)?;
            if self.current.is_empty() {
                return Ok(0);
            }
        }
        let available = &self.current[self.current_offset..];
        let take = available.len().min(output.len());
        output[..take].copy_from_slice(&available[..take]);
        self.current_offset += take;
        Ok(take)
    }
}

fn validation_io(error: ValidationFailure) -> io::Error {
    io::Error::new(
        if matches!(error, ValidationFailure::Incomplete) {
            io::ErrorKind::UnexpectedEof
        } else {
            io::ErrorKind::InvalidData
        },
        "backup validation failed",
    )
}

fn validate_manifest(
    header: &PackageHeader,
    manifest: &BackupManifest,
    observed: &[(u8, String, u64, [u8; 32])],
) -> Result<(), ValidationFailure> {
    if manifest.package_version != DNBAK_FORMAT_VERSION
        || manifest.backup_id != header.backup_id
        || manifest.local_day != header.local_day
        || manifest.created_at_ms != header.created_at_ms
        || manifest.source_schema_version != header.source_schema_version
        || manifest.body_schema_version != 1
        || observed.len() != 2 + manifest.assets.len()
    {
        return Err(ValidationFailure::Corrupt);
    }
    let database = observed.get(1).ok_or(ValidationFailure::Corrupt)?;
    if database.0 != 2
        || database.1 != manifest.database.path
        || database.2 != manifest.database.size
        || hex(&database.3) != manifest.database.sha256
    {
        return Err(ValidationFailure::Corrupt);
    }
    for (manifest_asset, observed_asset) in manifest.assets.iter().zip(observed.iter().skip(2)) {
        validate_note_id(&manifest_asset.asset_id).map_err(|_| ValidationFailure::Corrupt)?;
        if observed_asset.0 != 3
            || observed_asset.1 != manifest_asset.path
            || observed_asset.2 != manifest_asset.size
            || hex(&observed_asset.3) != manifest_asset.sha256_cipher
            || !is_canonical_asset_path_for_id(&manifest_asset.path, &manifest_asset.asset_id)
        {
            return Err(ValidationFailure::Corrupt);
        }
    }
    Ok(())
}

fn validate_component_path(kind: u8, index: u32, path: &str) -> Result<(), ValidationFailure> {
    if path.contains('\\') || path.starts_with('/') || path.contains("..") || path.contains(':') {
        return Err(ValidationFailure::Corrupt);
    }
    match (kind, index) {
        (1, 0) if path == MANIFEST_PATH => Ok(()),
        (2, 1) if path == DATABASE_PATH => Ok(()),
        (3, index) if index >= 2 && is_canonical_asset_path(path) => Ok(()),
        _ => Err(ValidationFailure::Corrupt),
    }
}

fn is_canonical_asset_path(path: &str) -> bool {
    let Some(relative) = path.strip_prefix("assets/") else {
        return false;
    };
    let mut components = relative.split('/');
    let (Some(first), Some(second), Some(filename), None) = (
        components.next(),
        components.next(),
        components.next(),
        components.next(),
    ) else {
        return false;
    };
    let Some(asset_id) = filename.strip_suffix(".dnimg") else {
        return false;
    };
    is_canonical_asset_path_for_id(path, asset_id)
        && first == &asset_id[0..2]
        && second == &asset_id[2..4]
}

fn is_canonical_asset_path_for_id(path: &str, asset_id: &str) -> bool {
    if validate_note_id(asset_id).is_err() || asset_id != asset_id.to_ascii_lowercase() {
        return false;
    }
    let expected = format!(
        "assets/{}/{}/{}.dnimg",
        &asset_id[0..2],
        &asset_id[2..4],
        asset_id
    );
    path == expected
}

fn create_extraction_file(root: &Path, component_path: &str) -> Result<File, ValidationFailure> {
    let destination = root.join(Path::new(component_path));
    let parent = destination.parent().ok_or(ValidationFailure::Corrupt)?;
    fs::create_dir_all(parent)?;
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(destination)
        .map_err(ValidationFailure::Io)
}

fn encode_header(header: &PackageHeader) -> Result<Vec<u8>, FoundationError> {
    validate_note_id(&header.backup_id).map_err(|_| FoundationError::backup_failed())?;
    validate_local_day(&header.local_day).map_err(|_| FoundationError::backup_failed())?;
    if header.created_at_ms < 0
        || header.payload_length == 0
        || header.component_count < 2
        || header.component_count > MAX_COMPONENTS
        || header.wrapped_root.is_empty()
        || header.wrapped_root.len() > MAX_WRAPPED_ROOT_LEN
    {
        return Err(FoundationError::backup_failed());
    }
    let mut encoded = Vec::with_capacity(128 + header.wrapped_root.len());
    encoded.extend_from_slice(DNBAK_MAGIC);
    encoded.extend_from_slice(&DNBAK_FORMAT_VERSION.to_le_bytes());
    encoded.push(DNBAK_ALGORITHM_AES_256_GCM);
    encoded.push(DNBAK_PROFILE_DPAPI_CURRENT_USER);
    encoded.extend_from_slice(&DNBAK_CHUNK_SIZE.to_le_bytes());
    encoded.extend_from_slice(&header.created_at_ms.to_le_bytes());
    encoded.extend_from_slice(&header.source_schema_version.to_le_bytes());
    encoded.extend_from_slice(&header.payload_length.to_le_bytes());
    encoded.extend_from_slice(&header.component_count.to_le_bytes());
    encoded.extend_from_slice(header.backup_id.as_bytes());
    encoded.extend_from_slice(header.local_day.as_bytes());
    encoded.extend_from_slice(&header.salt);
    encoded.extend_from_slice(&header.nonce_prefix);
    encoded.extend_from_slice(&(header.wrapped_root.len() as u32).to_le_bytes());
    encoded.extend_from_slice(&header.wrapped_root);
    Ok(encoded)
}

fn decode_header(input: &mut File) -> Result<PackageHeader, ValidationFailure> {
    let mut encoded = Vec::new();
    let magic = read_array_record::<8>(input, &mut encoded)?;
    if &magic != DNBAK_MAGIC {
        return Err(ValidationFailure::Corrupt);
    }
    let version = u16::from_le_bytes(read_array_record::<2>(input, &mut encoded)?);
    let algorithm = read_array_record::<1>(input, &mut encoded)?[0];
    let profile = read_array_record::<1>(input, &mut encoded)?[0];
    if version != DNBAK_FORMAT_VERSION
        || algorithm != DNBAK_ALGORITHM_AES_256_GCM
        || profile != DNBAK_PROFILE_DPAPI_CURRENT_USER
    {
        return Err(ValidationFailure::Unsupported);
    }
    let chunk_size = u32::from_le_bytes(read_array_record::<4>(input, &mut encoded)?);
    if chunk_size != DNBAK_CHUNK_SIZE {
        return Err(ValidationFailure::Unsupported);
    }
    let created_at_ms = i64::from_le_bytes(read_array_record::<8>(input, &mut encoded)?);
    let source_schema_version = u32::from_le_bytes(read_array_record::<4>(input, &mut encoded)?);
    let payload_length = u64::from_le_bytes(read_array_record::<8>(input, &mut encoded)?);
    let component_count = u32::from_le_bytes(read_array_record::<4>(input, &mut encoded)?);
    let backup_id =
        String::from_utf8(read_array_record::<BACKUP_ID_LEN>(input, &mut encoded)?.to_vec())
            .map_err(|_| ValidationFailure::Corrupt)?;
    let local_day =
        String::from_utf8(read_array_record::<LOCAL_DAY_LEN>(input, &mut encoded)?.to_vec())
            .map_err(|_| ValidationFailure::Corrupt)?;
    let salt = read_array_record::<DNBAK_SALT_LEN>(input, &mut encoded)?;
    let nonce_prefix = read_array_record::<DNBAK_NONCE_PREFIX_LEN>(input, &mut encoded)?;
    let wrapped_length = u32::from_le_bytes(read_array_record::<4>(input, &mut encoded)?) as usize;
    if created_at_ms < 0
        || payload_length == 0
        || !(2..=MAX_COMPONENTS).contains(&component_count)
        || wrapped_length == 0
        || wrapped_length > MAX_WRAPPED_ROOT_LEN
        || validate_note_id(&backup_id).is_err()
        || validate_local_day(&local_day).is_err()
    {
        return Err(ValidationFailure::Corrupt);
    }
    let mut wrapped_root = vec![0_u8; wrapped_length];
    input.read_exact(&mut wrapped_root)?;
    encoded.extend_from_slice(&wrapped_root);
    Ok(PackageHeader {
        backup_id,
        local_day,
        created_at_ms,
        source_schema_version,
        payload_length,
        component_count,
        salt,
        nonce_prefix,
        wrapped_root,
        encoded,
    })
}

fn read_array_record<const N: usize>(
    input: &mut File,
    encoded: &mut Vec<u8>,
) -> Result<[u8; N], ValidationFailure> {
    let mut bytes = [0_u8; N];
    input.read_exact(&mut bytes)?;
    encoded.extend_from_slice(&bytes);
    Ok(bytes)
}

fn discover_directory(
    directory: &Path,
    backup_key: &SecretKey,
) -> Result<Vec<BackupDiscovery>, FoundationError> {
    let entries = match fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(map_backup_io(error)),
    };
    let mut discoveries = Vec::new();
    for entry in entries {
        let entry = entry.map_err(map_backup_io)?;
        if !entry.file_type().map_err(map_backup_io)?.is_file()
            || entry.path().extension().and_then(|value| value.to_str()) != Some("dnbak")
        {
            continue;
        }
        match validate_package(&entry.path(), backup_key) {
            Ok(validated) => discoveries.push(BackupDiscovery {
                path: entry.path(),
                classification: BackupClassification::Valid,
                validated: Some(validated),
            }),
            Err(error) => discoveries.push(BackupDiscovery {
                path: entry.path(),
                classification: error.classification(),
                validated: None,
            }),
        }
    }
    discoveries.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(discoveries)
}

fn newest_valid(discoveries: &[BackupDiscovery]) -> Option<ValidatedBackup> {
    discoveries
        .iter()
        .filter_map(|entry| entry.validated.as_ref())
        .max_by(|left, right| {
            left.created_at_ms
                .cmp(&right.created_at_ms)
                .then_with(|| left.backup_id.cmp(&right.backup_id))
        })
        .cloned()
}

fn valid_backups(discoveries: &[BackupDiscovery]) -> Vec<ValidatedBackup> {
    discoveries
        .iter()
        .filter_map(|entry| entry.validated.clone())
        .collect()
}

fn health_from_discoveries(
    discoveries: &[BackupDiscovery],
    error: Option<ErrorCode>,
) -> BackupHealth {
    let newest = newest_valid(discoveries);
    let error = error.or_else(|| {
        discoveries
            .iter()
            .find_map(|entry| match entry.classification {
                BackupClassification::Valid => None,
                BackupClassification::Unsupported => Some(ErrorCode::BackupUnsupported),
                BackupClassification::Incomplete | BackupClassification::Corrupt => {
                    Some(ErrorCode::BackupCorrupted)
                }
            })
    });
    BackupHealth {
        state: if error.is_some() {
            BackupHealthState::Failed
        } else if newest.is_some() {
            BackupHealthState::Healthy
        } else {
            BackupHealthState::Never
        },
        last_success_at_ms: newest.as_ref().map(|backup| backup.created_at_ms),
        last_success_local_day: newest.as_ref().map(|backup| backup.local_day.clone()),
        valid_generation_count: discoveries
            .iter()
            .filter(|entry| entry.classification == BackupClassification::Valid)
            .count(),
        last_error_code: error,
    }
}

fn framed_length(path: &str, content_length: u64) -> Result<u64, FoundationError> {
    if path.is_empty() || path.len() > MAX_COMPONENT_PATH_LEN {
        return Err(FoundationError::backup_failed());
    }
    (1_u64 + 2 + path.len() as u64 + 8 + 32)
        .checked_add(content_length)
        .ok_or_else(FoundationError::backup_failed)
}

fn backup_filename(local_day: &str, backup_id: &str) -> Result<String, FoundationError> {
    validate_local_day(local_day).map_err(|_| FoundationError::backup_failed())?;
    validate_note_id(backup_id).map_err(|_| FoundationError::backup_failed())?;
    Ok(format!("auto-{local_day}-{backup_id}.dnbak"))
}

fn parse_backup_filename(filename: &str) -> Option<(String, String)> {
    let body = filename.strip_prefix("auto-")?.strip_suffix(".dnbak")?;
    if body.len() != LOCAL_DAY_LEN + 1 + BACKUP_ID_LEN {
        return None;
    }
    let local_day = &body[..LOCAL_DAY_LEN];
    let backup_id = &body[LOCAL_DAY_LEN + 1..];
    if body.as_bytes().get(LOCAL_DAY_LEN) != Some(&b'-')
        || validate_local_day(local_day).is_err()
        || validate_note_id(backup_id).is_err()
    {
        return None;
    }
    Some((local_day.to_owned(), backup_id.to_owned()))
}

fn validate_local_day(value: &str) -> Result<(), ()> {
    if value.len() != LOCAL_DAY_LEN
        || value.as_bytes()[4] != b'-'
        || value.as_bytes()[7] != b'-'
        || !value
            .bytes()
            .enumerate()
            .all(|(index, byte)| matches!(index, 4 | 7) || byte.is_ascii_digit())
    {
        return Err(());
    }
    let year: u32 = value[0..4].parse().map_err(|_| ())?;
    let month: u32 = value[5..7].parse().map_err(|_| ())?;
    let day: u32 = value[8..10].parse().map_err(|_| ())?;
    let leap_year =
        year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400));
    let max_day = match month {
        2 if leap_year => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        _ => return Err(()),
    };
    if year < 1970 || day == 0 || day > max_day {
        return Err(());
    }
    Ok(())
}

fn new_backup_id() -> Result<String, FoundationError> {
    let mut bytes = [0_u8; 16];
    getrandom::fill(&mut bytes).map_err(|_| FoundationError::backup_failed())?;
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    Ok(format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        bytes[0],
        bytes[1],
        bytes[2],
        bytes[3],
        bytes[4],
        bytes[5],
        bytes[6],
        bytes[7],
        bytes[8],
        bytes[9],
        bytes[10],
        bytes[11],
        bytes[12],
        bytes[13],
        bytes[14],
        bytes[15]
    ))
}

fn random_array<const N: usize>() -> Result<[u8; N], FoundationError> {
    let mut bytes = [0_u8; N];
    getrandom::fill(&mut bytes).map_err(|_| FoundationError::backup_failed())?;
    Ok(bytes)
}

fn derive_container_key(
    backup_key: &SecretKey,
    backup_id: &str,
    salt: &[u8; DNBAK_SALT_LEN],
) -> Result<Zeroizing<[u8; 32]>, FoundationError> {
    let hkdf = Hkdf::<Sha256>::new(Some(salt), backup_key.as_bytes());
    let mut info = Vec::with_capacity(DNBAK_CONTAINER_KEY_INFO.len() + backup_id.len());
    info.extend_from_slice(DNBAK_CONTAINER_KEY_INFO);
    info.extend_from_slice(backup_id.as_bytes());
    let mut key = Zeroizing::new([0_u8; 32]);
    hkdf.expand(&info, key.as_mut())
        .map_err(|_| FoundationError::backup_failed())?;
    Ok(key)
}

fn chunk_nonce(
    prefix: &[u8; DNBAK_NONCE_PREFIX_LEN],
    index: u32,
) -> Nonce<aes_gcm::aead::consts::U12> {
    let mut nonce = [0_u8; 12];
    nonce[..DNBAK_NONCE_PREFIX_LEN].copy_from_slice(prefix);
    nonce[DNBAK_NONCE_PREFIX_LEN..].copy_from_slice(&index.to_be_bytes());
    Nonce::from(nonce)
}

fn chunk_aad(
    header_hash: &[u8; 32],
    backup_id: &[u8],
    chunk_index: u32,
    plain_length: u32,
    total_plain_length: u64,
) -> Vec<u8> {
    let mut aad = Vec::with_capacity(32 + backup_id.len() + 16);
    aad.extend_from_slice(header_hash);
    aad.extend_from_slice(backup_id);
    aad.extend_from_slice(&chunk_index.to_le_bytes());
    aad.extend_from_slice(&plain_length.to_le_bytes());
    aad.extend_from_slice(&total_plain_length.to_le_bytes());
    aad
}

fn footer_aad(
    header_hash: &[u8; 32],
    backup_id: &[u8],
    chunk_count: u32,
    plain_length: u64,
    digest: &[u8; 32],
) -> Vec<u8> {
    let mut aad = Vec::with_capacity(32 + backup_id.len() + 8 + 4 + 8 + 32);
    aad.extend_from_slice(header_hash);
    aad.extend_from_slice(backup_id);
    aad.extend_from_slice(DNBAK_END_MAGIC);
    aad.extend_from_slice(&chunk_count.to_le_bytes());
    aad.extend_from_slice(&plain_length.to_le_bytes());
    aad.extend_from_slice(digest);
    aad
}

fn read_u8(input: &mut impl Read) -> Result<u8, ValidationFailure> {
    let mut bytes = [0_u8; 1];
    input.read_exact(&mut bytes)?;
    Ok(bytes[0])
}

fn read_u16(input: &mut impl Read) -> Result<u16, ValidationFailure> {
    let mut bytes = [0_u8; 2];
    input.read_exact(&mut bytes)?;
    Ok(u16::from_le_bytes(bytes))
}

fn read_u32(input: &mut impl Read) -> Result<u32, ValidationFailure> {
    let mut bytes = [0_u8; 4];
    input.read_exact(&mut bytes)?;
    Ok(u32::from_le_bytes(bytes))
}

fn read_u64(input: &mut impl Read) -> Result<u64, ValidationFailure> {
    let mut bytes = [0_u8; 8];
    input.read_exact(&mut bytes)?;
    Ok(u64::from_le_bytes(bytes))
}

fn digest_reader_validation(reader: &mut impl Read) -> Result<[u8; 32], ValidationFailure> {
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
    Ok(digest.finalize().into())
}

fn map_backup_io(error: io::Error) -> FoundationError {
    if error.raw_os_error() == Some(112) {
        FoundationError::new(ErrorCode::DiskFull, "The disk is full.")
    } else {
        FoundationError::backup_failed()
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    struct DiskFullWriter {
        bytes: Vec<u8>,
        remaining: usize,
    }

    impl Write for DiskFullWriter {
        fn write(&mut self, input: &[u8]) -> io::Result<usize> {
            if self.remaining == 0 {
                return Err(io::Error::from_raw_os_error(112));
            }
            let written = input.len().min(self.remaining);
            self.bytes.extend_from_slice(&input[..written]);
            self.remaining -= written;
            Ok(written)
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn disk_full_and_access_denied_keep_distinct_safe_error_classes() {
        assert_eq!(
            map_backup_io(io::Error::from_raw_os_error(112)).code(),
            ErrorCode::DiskFull
        );
        assert_eq!(
            map_backup_io(io::Error::from_raw_os_error(5)).code(),
            ErrorCode::BackupFailed
        );
    }

    #[test]
    fn disk_full_during_streaming_cannot_write_a_completion_footer() {
        let cipher = Aes256Gcm::new_from_slice(&[0x52; 32]).unwrap();
        let mut output = DiskFullWriter {
            bytes: Vec::new(),
            remaining: (DNBAK_CHUNK_SIZE as usize) + 512,
        };
        let payload = vec![0x91; (DNBAK_CHUNK_SIZE as usize) * 2];
        let mut writer = EncryptedPayloadWriter::new(
            &mut output,
            cipher,
            [0x23; 32],
            b"90000000-0000-4000-8000-000000000090",
            [0x35; DNBAK_NONCE_PREFIX_LEN],
            payload.len() as u64,
        );

        let error = writer.write_plain(&payload).unwrap_err();

        assert_eq!(error.code(), ErrorCode::DiskFull);
        assert!(!output.bytes.ends_with(DNBAK_END_MAGIC));
    }

    #[test]
    fn canonical_date_and_component_guards_reject_forged_paths() {
        assert!(validate_local_day("2024-02-29").is_ok());
        assert!(validate_local_day("2026-02-29").is_err());
        assert!(validate_local_day("2026-09-31").is_err());
        assert!(
            validate_component_path(
                3,
                2,
                "assets/90/00/90000000-0000-4000-8000-000000000002.dnimg"
            )
            .is_ok()
        );
        for path in [
            "../control/keyring.json",
            "assets/90/00/../../control/keyring.json",
            "assets/objects/90000000-0000-4000-8000-000000000002.dnimg",
            "assets/90/01/90000000-0000-4000-8000-000000000002.dnimg",
        ] {
            assert!(matches!(
                validate_component_path(3, 2, path),
                Err(ValidationFailure::Corrupt)
            ));
        }
    }

    #[test]
    fn cleanup_cannot_race_the_active_writer() {
        let manager = AutomaticBackupManager::new(
            PathBuf::from("unused-test-root"),
            SecretKey::from_bytes([0x83; 32]),
            vec![0x44; 16],
        )
        .unwrap();
        let _writer = manager.writer.lock().unwrap();
        assert_eq!(
            manager.cleanup_invalid(i64::MAX).unwrap_err().code(),
            ErrorCode::BackupBusy
        );
    }
}
