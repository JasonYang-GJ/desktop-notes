use std::{
    collections::{HashMap, HashSet},
    fs::{self, File, OpenOptions},
    io,
    path::{Path, PathBuf},
    sync::Mutex,
    time::{Duration, SystemTime},
};

use desktop_notes_core::{
    ASSET_GC_GRACE_MS, AssetRecord, AssetStore, DatabaseStatus, DateCount, EncryptedStore,
    ErrorCode, FoundationError, ImageSourceFormat, NewNote, NewTag, Note, NoteDateUpdate,
    NoteDeletion, NotePinUpdate, NoteRepository, NoteSummary, NoteTagUpdate, NoteUpdate,
    OrganizationRepository, SearchHit, SearchMatchTag, SearchRepository, SecretKey,
    ShortcutSettingsStore, ShortcutSpec, StagedAssetImport, Tag, TagRename,
    extract_image_asset_occurrences, gc_error_code, prepare_tag_name, validate_note_date,
    validate_note_id,
};
use rusqlite::{Connection, OpenFlags, OptionalExtension, params};
use zeroize::Zeroizing;

use crate::{MigrationRegistry, backup::ValidatedBackup, migrations::migration_failed};

const EXPECTED_CIPHER_VERSION: &str = "4.18.0 community";
const EXPECTED_SQLITE_VERSION: &str = "3.53.4";

struct OpenDatabase {
    connection: Connection,
    status: DatabaseStatus,
}

pub struct SqlCipherStore {
    path: PathBuf,
    app_version: String,
    registry: MigrationRegistry,
    open: Mutex<Option<OpenDatabase>>,
    staged_assets: Mutex<HashMap<String, StagedAssetImport>>,
}

#[derive(Clone, Debug)]
pub(crate) struct BackupSnapshotAsset {
    pub asset_id: String,
    pub storage_relpath: String,
    pub byte_size_cipher: u64,
}

#[derive(Clone, Debug)]
pub(crate) struct BackupSnapshotMetadata {
    pub schema_version: u32,
    pub assets: Vec<BackupSnapshotAsset>,
}

impl SqlCipherStore {
    pub fn new(path: PathBuf, app_version: impl Into<String>) -> Self {
        Self::with_registry(path, app_version, MigrationRegistry::b09())
    }

    pub fn with_registry(
        path: PathBuf,
        app_version: impl Into<String>,
        registry: MigrationRegistry,
    ) -> Self {
        Self {
            path,
            app_version: app_version.into(),
            registry,
            open: Mutex::new(None),
            staged_assets: Mutex::new(HashMap::new()),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn load_window_preferences(&self) -> Result<Option<String>, FoundationError> {
        self.with_connection(|connection| {
            connection
                .query_row(
                    "SELECT value_json FROM user_settings WHERE key = 'window_preferences_v1'",
                    [],
                    |row| row.get::<_, String>(0),
                )
                .optional()
                .map_err(map_repository_sqlite)
                .and_then(|value| match value {
                    Some(value) if value.len() <= 32 * 1024 => Ok(Some(value)),
                    Some(_) => Err(FoundationError::validation_failed()),
                    None => Ok(None),
                })
        })
    }

    pub fn save_window_preferences(
        &self,
        value_json: &str,
        updated_at_ms: i64,
    ) -> Result<(), FoundationError> {
        if value_json.len() > 32 * 1024 || updated_at_ms < 0 {
            return Err(FoundationError::validation_failed());
        }
        serde_json::from_str::<serde_json::Value>(value_json)
            .map_err(|_| FoundationError::validation_failed())?;
        self.with_connection(|connection| {
            connection
                .execute(
                    "INSERT INTO user_settings(key, value_json, updated_at_ms) \
                     VALUES ('window_preferences_v1', ?1, ?2) \
                     ON CONFLICT(key) DO UPDATE SET value_json = excluded.value_json, \
                       updated_at_ms = excluded.updated_at_ms",
                    params![value_json, updated_at_ms],
                )
                .map_err(map_repository_sqlite)?;
            Ok(())
        })
    }

    pub(crate) fn create_backup_snapshot(
        &self,
        destination: &Path,
        key: &SecretKey,
    ) -> Result<BackupSnapshotMetadata, FoundationError> {
        if destination.exists() {
            return Err(FoundationError::backup_failed());
        }
        let parent = destination.parent().ok_or_else(data_root_unavailable)?;
        fs::create_dir_all(parent).map_err(map_io)?;
        let snapshot_file = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(destination)
            .map_err(map_io)?;

        let result = self.with_connection(|source| {
            let mut snapshot = open_keyed(destination, key, true)?;
            {
                let backup = rusqlite::backup::Backup::new(source, &mut snapshot)
                    .map_err(|error| map_sqlite(error, ErrorCode::BackupFailed))?;
                backup
                    .run_to_completion(64, Duration::from_millis(2), None)
                    .map_err(|error| map_sqlite(error, ErrorCode::BackupFailed))?;
            }
            verify_cipher_integrity(&snapshot)?;
            let source_schema_version = schema_version(source)?;
            let snapshot_schema_version = schema_version(&snapshot)?;
            if snapshot_schema_version != source_schema_version {
                return Err(FoundationError::backup_corrupted());
            }

            let mut statement = source
                .prepare(
                    "SELECT DISTINCT a.id, a.storage_relpath, a.byte_size_cipher \
                     FROM assets a \
                     JOIN note_assets na ON na.asset_id = a.id \
                     JOIN notes n ON n.id = na.note_id \
                     WHERE a.state = 'ready' AND n.deleted_at_ms IS NULL \
                     ORDER BY a.storage_relpath",
                )
                .map_err(|error| map_sqlite(error, ErrorCode::BackupFailed))?;
            let assets = statement
                .query_map([], |row| {
                    Ok(BackupSnapshotAsset {
                        asset_id: row.get(0)?,
                        storage_relpath: row.get(1)?,
                        byte_size_cipher: row
                            .get::<_, i64>(2)?
                            .try_into()
                            .map_err(|_| rusqlite::Error::IntegralValueOutOfRange(2, i64::MAX))?,
                    })
                })
                .map_err(|error| map_sqlite(error, ErrorCode::BackupFailed))?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|error| map_sqlite(error, ErrorCode::BackupFailed))?;
            drop(statement);
            drop(snapshot);
            Ok(BackupSnapshotMetadata {
                schema_version: source_schema_version,
                assets,
            })
        });

        if result.is_err() {
            drop(snapshot_file);
            remove_staging_files(destination);
            return result;
        }
        snapshot_file.sync_all().map_err(map_io)?;
        drop(snapshot_file);
        result
    }

    pub(crate) fn cache_valid_backups(
        &self,
        backups: &[ValidatedBackup],
    ) -> Result<(), FoundationError> {
        self.with_connection(|connection| {
            let transaction = connection
                .transaction()
                .map_err(|error| map_sqlite(error, ErrorCode::BackupFailed))?;
            transaction
                .execute(
                    "UPDATE backup_catalog SET status = 'delete_pending' WHERE status = 'valid'",
                    [],
                )
                .map_err(|error| map_sqlite(error, ErrorCode::BackupFailed))?;
            for backup in backups {
                let package_size = i64::try_from(backup.package_size)
                    .map_err(|_| FoundationError::backup_failed())?;
                let package_sha256 = decode_hex_32(&backup.package_sha256_hex)?;
                transaction
                    .execute(
                        "INSERT INTO backup_catalog(
                           id, kind, storage_path, local_day, created_at_ms, completed_at_ms,
                           status, format_version, source_schema_version, package_size,
                           package_sha256, protection_profile, last_error_code
                         ) VALUES (?1, 'automatic', ?2, ?3, ?4, ?4, 'valid', 1, ?5, ?6, ?7,
                                   'automatic-dpapi-current-user', NULL)
                         ON CONFLICT(id) DO UPDATE SET
                           storage_path = excluded.storage_path,
                           local_day = excluded.local_day,
                           created_at_ms = excluded.created_at_ms,
                           completed_at_ms = excluded.completed_at_ms,
                           status = 'valid',
                           source_schema_version = excluded.source_schema_version,
                           package_size = excluded.package_size,
                           package_sha256 = excluded.package_sha256,
                           last_error_code = NULL",
                        params![
                            backup.backup_id,
                            backup.storage_path,
                            backup.local_day,
                            backup.created_at_ms,
                            backup.source_schema_version,
                            package_size,
                            package_sha256.as_slice(),
                        ],
                    )
                    .map_err(|error| map_sqlite(error, ErrorCode::BackupFailed))?;
            }
            transaction
                .commit()
                .map_err(|error| map_sqlite(error, ErrorCode::BackupFailed))
        })
    }

    fn initialize_fresh(&self, key: &SecretKey) -> Result<Connection, FoundationError> {
        let parent = self.path.parent().ok_or_else(data_root_unavailable)?;
        fs::create_dir_all(parent).map_err(map_io)?;
        let staging = staging_path(parent)?;
        let result = (|| {
            let mut connection = open_keyed(&staging, key, true)?;
            migrate(&mut connection, &self.registry, self.app_version.as_str())?;
            configure_runtime(&connection)?;
            connection
                .execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")
                .map_err(|error| map_sqlite(error, ErrorCode::DatabaseCorrupted))?;
            connection
                .close()
                .map_err(|(_, error)| map_sqlite(error, ErrorCode::DataRootUnavailable))?;
            if self.path.exists() {
                return Err(FoundationError::new(
                    ErrorCode::DataRootUnavailable,
                    "The encrypted database appeared during initialization.",
                ));
            }
            commit_staging_database(&staging, &self.path)?;
            open_keyed(&self.path, key, false)
        })();
        if result.is_err() {
            remove_staging_files(&staging);
        }
        result
    }

    fn open_existing(&self, key: &SecretKey) -> Result<Connection, FoundationError> {
        let mut connection = open_keyed(&self.path, key, false)?;
        migrate(&mut connection, &self.registry, self.app_version.as_str())?;
        configure_runtime(&connection)?;
        Ok(connection)
    }
}

fn commit_staging_database(staging: &Path, destination: &Path) -> Result<(), FoundationError> {
    commit_staging_database_with(staging, destination, |source, destination| {
        fs::hard_link(source, destination)
    })
}

fn commit_staging_database_with<Link>(
    staging: &Path,
    destination: &Path,
    link: Link,
) -> Result<(), FoundationError>
where
    Link: FnOnce(&Path, &Path) -> io::Result<()>,
{
    match link(staging, destination) {
        Ok(()) => {
            let _ = fs::remove_file(staging);
            return Ok(());
        }
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
            return Err(data_root_unavailable());
        }
        Err(_) => {}
    }

    persist_staging_database(staging, destination)?;
    let _ = fs::remove_file(staging);
    Ok(())
}

fn persist_staging_database(staging: &Path, destination: &Path) -> Result<(), FoundationError> {
    persist_staging_database_with(staging, destination, |source, destination_file| {
        io::copy(source, destination_file)
    })
}

fn persist_staging_database_with<CopyAction>(
    staging: &Path,
    destination: &Path,
    copy_action: CopyAction,
) -> Result<(), FoundationError>
where
    CopyAction: FnOnce(&mut File, &mut File) -> io::Result<u64>,
{
    let mut source = File::open(staging).map_err(map_io)?;
    let expected_length = source.metadata().map_err(map_io)?.len();
    let mut exclusive = ExclusiveDestination::create(destination).map_err(map_io)?;
    let copied = copy_action(&mut source, exclusive.file_mut()).map_err(map_io)?;
    exclusive.sync_close_and_verify_length(copied, expected_length)
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

    fn sync_close_and_verify_length(
        self,
        copied: u64,
        expected: u64,
    ) -> Result<(), FoundationError> {
        self.sync_close_and_verify_length_with(
            copied,
            expected,
            |file| file.sync_all(),
            |path| fs::metadata(path).map(|metadata| metadata.len()),
        )
    }

    fn sync_close_and_verify_length_with<SyncAction, LengthAction>(
        mut self,
        copied: u64,
        expected: u64,
        sync_action: SyncAction,
        length_action: LengthAction,
    ) -> Result<(), FoundationError>
    where
        SyncAction: FnOnce(&File) -> io::Result<()>,
        LengthAction: FnOnce(&Path) -> io::Result<u64>,
    {
        let file = self.file.take().expect("exclusive file remains open");
        sync_action(&file).map_err(map_io)?;
        drop(file);
        if copied != expected || length_action(self.path).map_err(map_io)? != expected {
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

impl EncryptedStore for SqlCipherStore {
    fn exists(&self) -> Result<bool, FoundationError> {
        match fs::metadata(&self.path) {
            Ok(metadata) if metadata.is_file() => Ok(true),
            Ok(_) => Err(data_root_unavailable()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
            Err(error) => Err(map_io(error)),
        }
    }

    fn open_or_initialize(&self, key: &SecretKey) -> Result<DatabaseStatus, FoundationError> {
        let mut open = self.open.lock().map_err(|_| {
            FoundationError::new(
                ErrorCode::InternalError,
                "The database connection state is unavailable.",
            )
        })?;
        if let Some(database) = open.as_ref() {
            return Ok(database.status.clone());
        }

        let existed = self.exists()?;
        let connection = if existed {
            self.open_existing(key)?
        } else {
            self.initialize_fresh(key)?
        };
        let schema_version = schema_version(&connection)?;
        let cipher_version = cipher_version(&connection)?;
        let status = DatabaseStatus {
            newly_created: !existed,
            schema_version,
            cipher_version,
        };
        *open = Some(OpenDatabase {
            connection,
            status: status.clone(),
        });
        Ok(status)
    }
}

impl NoteRepository for SqlCipherStore {
    fn create(&self, input: NewNote) -> Result<Note, FoundationError> {
        let note = Note::from_new(input);
        note.validate_stored()?;
        let asset_occurrences = extract_image_asset_occurrences(&note.body_json)?;
        self.with_connection(|connection| {
            let transaction = connection.transaction().map_err(map_repository_sqlite)?;
            transaction
                .execute(
                    "INSERT INTO notes(\
                       id, note_date, title, body_json, body_format, body_schema_version,\
                       body_text, content_hash, created_at_ms, updated_at_ms, revision\
                     ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?9, 1)",
                    params![
                        note.id,
                        note.note_date,
                        note.title,
                        note.body_json,
                        note.body_format,
                        note.body_schema_version,
                        note.body_text,
                        note.content_hash.as_slice(),
                        note.created_at_ms,
                    ],
                )
                .map_err(map_repository_sqlite)?;
            let committed_staged_assets = if self.registry.latest_version() >= 5 {
                synchronize_note_assets(
                    &transaction,
                    &self.staged_assets,
                    &note.id,
                    &asset_occurrences,
                    note.created_at_ms,
                )?
            } else if asset_occurrences.is_empty() {
                HashSet::new()
            } else {
                return Err(FoundationError::asset_not_found());
            };
            transaction.commit().map_err(map_repository_sqlite)?;
            if !committed_staged_assets.is_empty() {
                let mut staged = self
                    .staged_assets
                    .lock()
                    .map_err(|_| repository_unavailable())?;
                staged.retain(|_, value| !committed_staged_assets.contains(&value.record.asset_id));
            }
            Ok(note)
        })
    }

    fn get(&self, id: &str) -> Result<Option<Note>, FoundationError> {
        self.with_connection(|connection| read_note(connection, id))
    }

    fn list_for_date(&self, note_date: &str) -> Result<Vec<NoteSummary>, FoundationError> {
        self.with_connection(|connection| {
            let mut statement = connection
                .prepare(
                    "SELECT id, note_date, title, body_json, body_format, body_schema_version, \
                            body_text, content_hash, created_at_ms, updated_at_ms, revision, is_pinned \
                       FROM notes \
                      WHERE note_date = ?1 AND deleted_at_ms IS NULL AND archived_at_ms IS NULL \
                      ORDER BY is_pinned DESC, updated_at_ms DESC, row_id DESC",
                )
                .map_err(map_repository_sqlite)?;
            let stored = statement
                .query_map([note_date], StoredNoteRow::from_row)
                .map_err(map_repository_sqlite)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(map_repository_sqlite)?;
            stored
                .into_iter()
                .map(StoredNoteRow::into_note)
                .map(|note| note.map(|note| NoteSummary::from(&note)))
                .collect()
        })
    }

    fn count_by_date_range(
        &self,
        start_date: &str,
        end_date_exclusive: &str,
    ) -> Result<Vec<DateCount>, FoundationError> {
        self.with_connection(|connection| {
            let mut statement = connection
                .prepare(
                    "SELECT note_date, COUNT(*) \
                       FROM notes \
                      WHERE note_date >= ?1 AND note_date < ?2 \
                        AND deleted_at_ms IS NULL AND archived_at_ms IS NULL \
                      GROUP BY note_date \
                      ORDER BY note_date ASC",
                )
                .map_err(map_repository_sqlite)?;
            statement
                .query_map(params![start_date, end_date_exclusive], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
                })
                .map_err(map_repository_sqlite)?
                .map(|row| {
                    let (note_date, count) = row.map_err(map_repository_sqlite)?;
                    let count = count.try_into().map_err(|_| database_corrupted())?;
                    Ok(DateCount { note_date, count })
                })
                .collect()
        })
    }

    fn update_content(&self, input: NoteUpdate) -> Result<Note, FoundationError> {
        let base_revision =
            i64::try_from(input.base_revision).map_err(|_| FoundationError::validation_failed())?;
        let asset_occurrences = extract_image_asset_occurrences(&input.body_json)?;
        self.with_connection(|connection| {
            let transaction = connection.transaction().map_err(map_repository_sqlite)?;
            let affected = transaction
                .execute(
                    "UPDATE notes \
                        SET title = ?1, body_json = ?2, body_text = ?3, content_hash = ?4, \
                            updated_at_ms = ?5, revision = revision + 1 \
                      WHERE id = ?6 AND revision = ?7",
                    params![
                        input.title,
                        input.body_json,
                        input.body_text,
                        input.content_hash.as_slice(),
                        input.updated_at_ms,
                        input.id,
                        base_revision,
                    ],
                )
                .map_err(map_repository_sqlite)?;
            if affected == 0 {
                let exists = transaction
                    .query_row("SELECT 1 FROM notes WHERE id = ?1", [&input.id], |_| Ok(()))
                    .optional()
                    .map_err(map_repository_sqlite)?
                    .is_some();
                return Err(if exists {
                    FoundationError::revision_conflict()
                } else {
                    FoundationError::note_not_found()
                });
            }
            let committed_staged_assets = if self.registry.latest_version() >= 5 {
                synchronize_note_assets(
                    &transaction,
                    &self.staged_assets,
                    &input.id,
                    &asset_occurrences,
                    input.updated_at_ms,
                )?
            } else if asset_occurrences.is_empty() {
                HashSet::new()
            } else {
                return Err(FoundationError::asset_not_found());
            };
            let updated =
                read_note(&transaction, &input.id)?.ok_or_else(FoundationError::note_not_found)?;
            transaction.commit().map_err(map_repository_sqlite)?;
            if !committed_staged_assets.is_empty() {
                let mut staged = self
                    .staged_assets
                    .lock()
                    .map_err(|_| repository_unavailable())?;
                staged.retain(|_, value| !committed_staged_assets.contains(&value.record.asset_id));
            }
            Ok(updated)
        })
    }

    fn update_date(&self, input: NoteDateUpdate) -> Result<Note, FoundationError> {
        let base_revision =
            i64::try_from(input.base_revision).map_err(|_| FoundationError::validation_failed())?;
        self.with_connection(|connection| {
            let transaction = connection.transaction().map_err(map_repository_sqlite)?;
            let affected = transaction
                .execute(
                    "UPDATE notes \
                        SET note_date = ?1, content_hash = ?2, updated_at_ms = ?3, \
                            revision = revision + 1 \
                      WHERE id = ?4 AND revision = ?5",
                    params![
                        input.note_date,
                        input.content_hash.as_slice(),
                        input.updated_at_ms,
                        input.id,
                        base_revision,
                    ],
                )
                .map_err(map_repository_sqlite)?;
            if affected == 0 {
                let exists = transaction
                    .query_row("SELECT 1 FROM notes WHERE id = ?1", [&input.id], |_| Ok(()))
                    .optional()
                    .map_err(map_repository_sqlite)?
                    .is_some();
                return Err(if exists {
                    FoundationError::revision_conflict()
                } else {
                    FoundationError::note_not_found()
                });
            }
            let updated =
                read_note(&transaction, &input.id)?.ok_or_else(FoundationError::note_not_found)?;
            transaction.commit().map_err(map_repository_sqlite)?;
            Ok(updated)
        })
    }

    fn delete(&self, input: NoteDeletion) -> Result<(), FoundationError> {
        let base_revision =
            i64::try_from(input.base_revision).map_err(|_| FoundationError::validation_failed())?;
        self.with_connection(|connection| {
            let transaction = connection.transaction().map_err(map_repository_sqlite)?;
            let affected = transaction
                .execute(
                    "UPDATE notes \
                        SET deleted_at_ms = ?1, updated_at_ms = ?1, revision = revision + 1, \
                            is_pinned = 0 \
                      WHERE id = ?2 AND revision = ?3 AND deleted_at_ms IS NULL",
                    params![input.deleted_at_ms, input.id, base_revision],
                )
                .map_err(map_repository_sqlite)?;
            if affected == 0 {
                let active = transaction
                    .query_row(
                        "SELECT 1 FROM notes WHERE id = ?1 AND deleted_at_ms IS NULL",
                        [&input.id],
                        |_| Ok(()),
                    )
                    .optional()
                    .map_err(map_repository_sqlite)?
                    .is_some();
                return Err(if active {
                    FoundationError::revision_conflict()
                } else {
                    FoundationError::note_not_found()
                });
            }
            transaction.commit().map_err(map_repository_sqlite)?;
            Ok(())
        })
    }
}

fn synchronize_note_assets(
    transaction: &rusqlite::Transaction<'_>,
    staged_assets: &Mutex<HashMap<String, StagedAssetImport>>,
    note_id: &str,
    asset_occurrences: &[String],
    updated_at_ms: i64,
) -> Result<HashSet<String>, FoundationError> {
    let old_asset_ids = {
        let mut statement = transaction
            .prepare("SELECT DISTINCT asset_id FROM note_assets WHERE note_id = ?1")
            .map_err(map_repository_sqlite)?;
        statement
            .query_map([note_id], |row| row.get::<_, String>(0))
            .map_err(map_repository_sqlite)?
            .collect::<Result<HashSet<_>, _>>()
            .map_err(map_repository_sqlite)?
    };
    let desired_asset_ids = asset_occurrences.iter().cloned().collect::<HashSet<_>>();
    let staged = staged_assets.lock().map_err(|_| repository_unavailable())?;
    let mut committed_staged_assets = HashSet::new();

    for asset_id in &desired_asset_ids {
        validate_note_id(asset_id)?;
        if read_asset(transaction, asset_id)?.is_none() {
            let staged_asset = staged
                .values()
                .find(|candidate| candidate.record.asset_id == *asset_id)
                .ok_or_else(FoundationError::asset_not_found)?;
            staged_asset.record.validate()?;
            insert_asset(transaction, &staged_asset.record)?;
            committed_staged_assets.insert(asset_id.clone());
        }
        transaction
            .execute(
                "UPDATE assets SET state = 'ready' WHERE id = ?1",
                [asset_id],
            )
            .map_err(map_repository_sqlite)?;
        transaction
            .execute("DELETE FROM asset_gc_queue WHERE asset_id = ?1", [asset_id])
            .map_err(map_repository_sqlite)?;
    }
    drop(staged);

    transaction
        .execute("DELETE FROM note_assets WHERE note_id = ?1", [note_id])
        .map_err(map_repository_sqlite)?;
    for (sort_order, asset_id) in asset_occurrences.iter().enumerate() {
        let occurrence_id = format!("{asset_id}:{sort_order}");
        transaction
            .execute(
                "INSERT INTO note_assets(note_id, asset_id, occurrence_id, sort_order, created_at_ms) \
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![note_id, asset_id, occurrence_id, sort_order as i64, updated_at_ms],
            )
            .map_err(map_repository_sqlite)?;
    }

    let not_before_ms = updated_at_ms
        .checked_add(ASSET_GC_GRACE_MS)
        .ok_or_else(FoundationError::validation_failed)?;
    for removed_asset_id in old_asset_ids.difference(&desired_asset_ids) {
        let reference_count: i64 = transaction
            .query_row(
                "SELECT COUNT(*) FROM note_assets WHERE asset_id = ?1",
                [removed_asset_id],
                |row| row.get(0),
            )
            .map_err(map_repository_sqlite)?;
        if reference_count == 0 {
            transaction
                .execute(
                    "UPDATE assets SET state = 'gc_pending' WHERE id = ?1",
                    [removed_asset_id],
                )
                .map_err(map_repository_sqlite)?;
            transaction
                .execute(
                    "INSERT INTO asset_gc_queue(asset_id, not_before_ms, attempts, last_error_code) \
                     VALUES (?1, ?2, 0, NULL) \
                     ON CONFLICT(asset_id) DO UPDATE SET not_before_ms = excluded.not_before_ms, \
                       attempts = 0, last_error_code = NULL",
                    params![removed_asset_id, not_before_ms],
                )
                .map_err(map_repository_sqlite)?;
        }
    }
    Ok(committed_staged_assets)
}

fn insert_asset(connection: &Connection, record: &AssetRecord) -> Result<(), FoundationError> {
    connection
        .execute(
            "INSERT INTO assets(\
               id, media_type, source_format, storage_relpath, byte_size_plain, byte_size_cipher, \
               pixel_width, pixel_height, sha256_plain, crypto_format_version, key_id, state, created_at_ms\
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, 'ready', ?12)",
            params![
                record.asset_id,
                record.media_type,
                record.source_format.as_str(),
                record.storage_relpath,
                i64::try_from(record.byte_size_plain)
                    .map_err(|_| FoundationError::validation_failed())?,
                i64::try_from(record.byte_size_cipher)
                    .map_err(|_| FoundationError::validation_failed())?,
                i64::from(record.pixel_width),
                i64::from(record.pixel_height),
                record.sha256_plain.as_slice(),
                i64::from(record.crypto_format_version),
                record.key_id,
                record.created_at_ms,
            ],
        )
        .map_err(map_repository_sqlite)?;
    Ok(())
}

impl AssetStore for SqlCipherStore {
    fn find_staged_import(
        &self,
        client_import_id: &str,
    ) -> Result<Option<StagedAssetImport>, FoundationError> {
        validate_note_id(client_import_id)?;
        let staged = self
            .staged_assets
            .lock()
            .map_err(|_| repository_unavailable())?;
        Ok(staged
            .values()
            .find(|value| value.client_import_id == client_import_id)
            .cloned())
    }

    fn stage_import(&self, staged_import: StagedAssetImport) -> Result<(), FoundationError> {
        validate_note_id(&staged_import.client_import_id)?;
        staged_import.record.validate()?;
        let mut staged = self
            .staged_assets
            .lock()
            .map_err(|_| repository_unavailable())?;
        if staged.len() >= 128 {
            return Err(FoundationError::new(
                ErrorCode::InternalError,
                "Too many image imports are awaiting a Note save.",
            ));
        }
        if staged.values().any(|value| {
            value.client_import_id == staged_import.client_import_id
                || value.record.asset_id == staged_import.record.asset_id
        }) {
            return Err(FoundationError::validation_failed());
        }
        staged.insert(staged_import.record.asset_id.clone(), staged_import);
        Ok(())
    }

    fn discard_staged_import(
        &self,
        client_import_id: &str,
    ) -> Result<Option<StagedAssetImport>, FoundationError> {
        validate_note_id(client_import_id)?;
        let mut staged = self
            .staged_assets
            .lock()
            .map_err(|_| repository_unavailable())?;
        let asset_id = staged
            .iter()
            .find(|(_, value)| value.client_import_id == client_import_id)
            .map(|(asset_id, _)| asset_id.clone());
        Ok(asset_id.and_then(|asset_id| staged.remove(&asset_id)))
    }

    fn get_ready_asset_for_note(
        &self,
        note_id: &str,
        asset_id: &str,
    ) -> Result<Option<AssetRecord>, FoundationError> {
        validate_note_id(note_id)?;
        validate_note_id(asset_id)?;
        self.with_connection(|connection| {
            connection
                .query_row(
                    "SELECT a.id, a.media_type, a.source_format, a.storage_relpath, \
                            a.byte_size_plain, a.byte_size_cipher, a.pixel_width, a.pixel_height, \
                            a.sha256_plain, a.crypto_format_version, a.key_id, a.created_at_ms \
                       FROM assets a \
                      WHERE a.id = ?1 AND a.state = 'ready' \
                        AND EXISTS (SELECT 1 FROM note_assets na WHERE na.note_id = ?2 AND na.asset_id = a.id)",
                    params![asset_id, note_id],
                    StoredAssetRow::from_row,
                )
                .optional()
                .map_err(map_repository_sqlite)?
                .map(StoredAssetRow::into_record)
                .transpose()
        })
    }

    fn tracked_storage_relpaths(&self) -> Result<Vec<String>, FoundationError> {
        self.with_connection(|connection| {
            let mut statement = connection
                .prepare("SELECT storage_relpath FROM assets ORDER BY storage_relpath")
                .map_err(map_repository_sqlite)?;
            statement
                .query_map([], |row| row.get::<_, String>(0))
                .map_err(map_repository_sqlite)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(map_repository_sqlite)
        })
    }

    fn list_due_gc_assets(
        &self,
        now_ms: i64,
        limit: u32,
    ) -> Result<Vec<AssetRecord>, FoundationError> {
        if now_ms < 0 || !(1..=256).contains(&limit) {
            return Err(FoundationError::validation_failed());
        }
        self.with_connection(|connection| {
            let mut statement = connection
                .prepare(
                    "SELECT a.id, a.media_type, a.source_format, a.storage_relpath, \
                            a.byte_size_plain, a.byte_size_cipher, a.pixel_width, a.pixel_height, \
                            a.sha256_plain, a.crypto_format_version, a.key_id, a.created_at_ms \
                       FROM assets a JOIN asset_gc_queue q ON q.asset_id = a.id \
                      WHERE a.state = 'gc_pending' AND q.not_before_ms <= ?1 \
                        AND NOT EXISTS (SELECT 1 FROM note_assets na WHERE na.asset_id = a.id) \
                      ORDER BY q.not_before_ms, a.id LIMIT ?2",
                )
                .map_err(map_repository_sqlite)?;
            statement
                .query_map(params![now_ms, limit], StoredAssetRow::from_row)
                .map_err(map_repository_sqlite)?
                .map(|row| row.map_err(map_repository_sqlite)?.into_record())
                .collect()
        })
    }

    fn finalize_gc_asset(&self, asset_id: &str) -> Result<bool, FoundationError> {
        validate_note_id(asset_id)?;
        self.with_connection(|connection| {
            let affected = connection
                .execute(
                    "DELETE FROM assets WHERE id = ?1 AND state = 'gc_pending' \
                       AND NOT EXISTS (SELECT 1 FROM note_assets WHERE asset_id = ?1)",
                    [asset_id],
                )
                .map_err(map_repository_sqlite)?;
            Ok(affected == 1)
        })
    }

    fn record_gc_failure(
        &self,
        asset_id: &str,
        error_code: ErrorCode,
    ) -> Result<(), FoundationError> {
        validate_note_id(asset_id)?;
        self.with_connection(|connection| {
            connection
                .execute(
                    "UPDATE asset_gc_queue SET attempts = attempts + 1, last_error_code = ?1 \
                      WHERE asset_id = ?2",
                    params![gc_error_code(error_code), asset_id],
                )
                .map_err(map_repository_sqlite)?;
            Ok(())
        })
    }
}

impl ShortcutSettingsStore for SqlCipherStore {
    fn load_quick_capture_shortcut(&self) -> Result<Option<String>, FoundationError> {
        self.with_connection(|connection| {
            let value: Option<String> = connection
                .query_row(
                    "SELECT value_json FROM user_settings WHERE key = 'quick_capture_shortcut'",
                    [],
                    |row| row.get(0),
                )
                .optional()
                .map_err(map_repository_sqlite)?;
            value
                .map(|value| {
                    serde_json::from_str::<String>(&value)
                        .map_err(|_| FoundationError::validation_failed())
                })
                .transpose()
        })
    }

    fn save_quick_capture_shortcut(
        &self,
        shortcut: &ShortcutSpec,
        updated_at_ms: i64,
    ) -> Result<(), FoundationError> {
        if updated_at_ms < 0 {
            return Err(FoundationError::validation_failed());
        }
        let value = serde_json::to_string(shortcut.as_str())
            .map_err(|_| FoundationError::validation_failed())?;
        self.with_connection(|connection| {
            connection
                .execute(
                    "INSERT INTO user_settings(key, value_json, updated_at_ms) \
                     VALUES ('quick_capture_shortcut', ?1, ?2) \
                     ON CONFLICT(key) DO UPDATE SET value_json = excluded.value_json, \
                       updated_at_ms = excluded.updated_at_ms",
                    params![value, updated_at_ms],
                )
                .map_err(map_repository_sqlite)?;
            Ok(())
        })
    }
}

impl OrganizationRepository for SqlCipherStore {
    fn get_note_for_metadata(&self, note_id: &str) -> Result<Option<Note>, FoundationError> {
        self.with_connection(|connection| read_note(connection, note_id))
    }

    fn list_tags(&self) -> Result<Vec<Tag>, FoundationError> {
        self.with_connection(|connection| {
            let mut statement = connection
                .prepare(
                    "SELECT id, name, normalized_name, is_seed_default, created_at_ms, updated_at_ms \
                       FROM tags ORDER BY normalized_name ASC, id ASC",
                )
                .map_err(map_repository_sqlite)?;
            statement
                .query_map([], StoredTagRow::from_row)
                .map_err(map_repository_sqlite)?
                .map(|row| row.map_err(map_repository_sqlite)?.into_tag())
                .collect()
        })
    }

    fn find_tag_by_normalized_name(
        &self,
        normalized_name: &str,
    ) -> Result<Option<Tag>, FoundationError> {
        self.with_connection(|connection| read_tag_by_normalized_name(connection, normalized_name))
    }

    fn create_tag(&self, input: NewTag) -> Result<Tag, FoundationError> {
        let tag = Tag::from_new(input);
        tag.validate_stored()?;
        self.with_connection(|connection| {
            connection
                .execute(
                    "INSERT INTO tags(\
                       id, name, normalized_name, is_seed_default, created_at_ms, updated_at_ms\
                     ) VALUES (?1, ?2, ?3, 0, ?4, ?4)",
                    params![tag.id, tag.name, tag.normalized_name, tag.created_at_ms],
                )
                .map_err(map_tag_sqlite)?;
            Ok(tag)
        })
    }

    fn rename_tag(&self, input: TagRename) -> Result<Tag, FoundationError> {
        self.with_connection(|connection| {
            let affected = connection
                .execute(
                    "UPDATE tags SET name = ?1, normalized_name = ?2, updated_at_ms = ?3 \
                      WHERE id = ?4",
                    params![
                        input.name,
                        input.normalized_name,
                        input.updated_at_ms,
                        input.id
                    ],
                )
                .map_err(map_tag_sqlite)?;
            if affected == 0 {
                return Err(FoundationError::tag_not_found());
            }
            read_tag(connection, &input.id)?.ok_or_else(FoundationError::tag_not_found)
        })
    }

    fn delete_tag(&self, tag_id: &str) -> Result<(), FoundationError> {
        self.with_connection(|connection| {
            let transaction = connection.transaction().map_err(map_repository_sqlite)?;
            let affected = transaction
                .execute("DELETE FROM tags WHERE id = ?1", [tag_id])
                .map_err(map_repository_sqlite)?;
            if affected == 0 {
                return Err(FoundationError::tag_not_found());
            }
            transaction.commit().map_err(map_repository_sqlite)
        })
    }

    fn list_tags_for_note(&self, note_id: &str) -> Result<Vec<Tag>, FoundationError> {
        self.with_connection(|connection| {
            if read_note(connection, note_id)?.is_none() {
                return Err(FoundationError::note_not_found());
            }
            read_tags_for_note(connection, note_id)
        })
    }

    fn assign_tag(&self, input: NoteTagUpdate) -> Result<Note, FoundationError> {
        self.with_connection(|connection| mutate_note_tag(connection, input, true))
    }

    fn remove_tag(&self, input: NoteTagUpdate) -> Result<Note, FoundationError> {
        self.with_connection(|connection| mutate_note_tag(connection, input, false))
    }

    fn set_pinned(&self, input: NotePinUpdate) -> Result<Note, FoundationError> {
        let base_revision =
            i64::try_from(input.base_revision).map_err(|_| FoundationError::validation_failed())?;
        self.with_connection(|connection| {
            let transaction = connection.transaction().map_err(map_repository_sqlite)?;
            let current = read_note(&transaction, &input.note_id)?
                .ok_or_else(FoundationError::note_not_found)?;
            if current.revision != input.base_revision {
                return Err(FoundationError::revision_conflict());
            }
            if current.is_pinned != input.is_pinned {
                transaction
                    .execute(
                        "UPDATE notes SET is_pinned = ?1, updated_at_ms = ?2, revision = revision + 1 \
                          WHERE id = ?3 AND revision = ?4",
                        params![input.is_pinned, input.updated_at_ms, input.note_id, base_revision],
                    )
                    .map_err(map_repository_sqlite)?;
            }
            let updated = read_note(&transaction, &input.note_id)?
                .ok_or_else(FoundationError::note_not_found)?;
            transaction.commit().map_err(map_repository_sqlite)?;
            Ok(updated)
        })
    }

    fn list_recent(&self, limit: u32) -> Result<Vec<NoteSummary>, FoundationError> {
        self.with_connection(|connection| {
            let mut statement = connection
                .prepare(
                    "SELECT id, note_date, title, body_json, body_format, body_schema_version, \
                            body_text, content_hash, created_at_ms, updated_at_ms, revision, is_pinned \
                       FROM notes \
                      WHERE deleted_at_ms IS NULL AND archived_at_ms IS NULL \
                      ORDER BY updated_at_ms DESC, row_id DESC LIMIT ?1",
                )
                .map_err(map_repository_sqlite)?;
            let stored = statement
                .query_map([i64::from(limit)], StoredNoteRow::from_row)
                .map_err(map_repository_sqlite)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(map_repository_sqlite)?;
            stored
                .into_iter()
                .map(StoredNoteRow::into_note)
                .map(|note| note.map(|note| NoteSummary::from(&note)))
                .collect()
        })
    }
}

impl SearchRepository for SqlCipherStore {
    fn search(&self, query: &str, limit: u32) -> Result<Vec<SearchHit>, FoundationError> {
        self.with_connection(|connection| {
            let short_query = query.chars().count() < 3;
            let escaped_like = escaped_like_pattern(query);
            let search_term = if short_query {
                escaped_like.clone()
            } else {
                format!("\"{}\"", query.replace('"', "\"\""))
            };
            let sql = if short_query {
                r#"SELECT n.id,n.note_date,n.title,n.body_text,n.updated_at_ms,n.revision,n.is_pinned
                   FROM notes n
                  WHERE n.deleted_at_ms IS NULL
                    AND (n.title LIKE ?1 ESCAPE '\'
                         OR n.body_text LIKE ?1 ESCAPE '\'
                         OR EXISTS(
                             SELECT 1 FROM note_tags nt JOIN tags t ON t.id=nt.tag_id
                              WHERE nt.note_id=n.id AND t.name LIKE ?1 ESCAPE '\'
                         ))
                  ORDER BY CASE
                               WHEN n.title LIKE ?1 ESCAPE '\' THEN 0
                               WHEN n.body_text LIKE ?1 ESCAPE '\' THEN 1
                               ELSE 2
                           END,
                           n.updated_at_ms DESC,n.row_id DESC
                  LIMIT ?2"#
            } else {
                r#"SELECT n.id,n.note_date,n.title,n.body_text,n.updated_at_ms,n.revision,n.is_pinned
                   FROM note_fts JOIN notes n ON n.id=note_fts.note_id
                  WHERE n.deleted_at_ms IS NULL AND note_fts MATCH ?1
                  ORDER BY bm25(note_fts,0.0,10.0,1.0,5.0),n.updated_at_ms DESC,n.row_id DESC
                  LIMIT ?2"#
            };
            let mut statement = connection.prepare(sql).map_err(map_repository_sqlite)?;
            let rows = statement
                .query_map(params![search_term, i64::from(limit.min(100))], SearchRow::from_row)
                .map_err(map_repository_sqlite)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(map_repository_sqlite)?;
            drop(statement);

            let mut tag_statement = connection
                .prepare(
                    "SELECT t.id,t.name FROM note_tags nt JOIN tags t ON t.id=nt.tag_id \
                     WHERE nt.note_id=?1 AND t.name LIKE ?2 ESCAPE '\\' \
                     ORDER BY t.normalized_name,t.id",
                )
                .map_err(map_repository_sqlite)?;
            rows.into_iter()
                .map(|row| {
                    let matching_tags = tag_statement
                        .query_map(params![&row.id, &escaped_like], |tag| {
                            Ok((tag.get::<_, String>(0)?, tag.get::<_, String>(1)?))
                        })
                        .map_err(map_repository_sqlite)?
                        .collect::<Result<Vec<_>, _>>()
                        .map_err(map_repository_sqlite)?;
                    let matching_tags = matching_tags
                        .into_iter()
                        .map(|(id, name)| {
                            validate_note_id(&id)?;
                            let prepared = prepare_tag_name(&name)?;
                            if prepared.name != name {
                                return Err(FoundationError::validation_failed());
                            }
                            Ok(SearchMatchTag { id, name })
                        })
                        .collect::<Result<Vec<_>, _>>()?;
                    row.into_hit(query, matching_tags)
                })
                .collect()
        })
    }

    fn rebuild_index(&self) -> Result<(), FoundationError> {
        self.with_connection(|connection| {
            let transaction = connection.transaction().map_err(map_repository_sqlite)?;
            transaction
                .execute("DELETE FROM note_fts", [])
                .map_err(map_repository_sqlite)?;
            transaction
                .execute(
                    r#"INSERT INTO note_fts(title,body_text,tags_text,note_id)
                       SELECT n.title,n.body_text,
                              COALESCE((SELECT group_concat(t.name,' ')
                                          FROM note_tags nt
                                          JOIN tags t ON t.id=nt.tag_id
                                         WHERE nt.note_id=n.id),'') AS tags_text,
                              n.id
                         FROM notes n
                        WHERE n.deleted_at_ms IS NULL"#,
                    [],
                )
                .map_err(map_repository_sqlite)?;
            transaction.commit().map_err(map_repository_sqlite)
        })
    }
}

struct SearchRow {
    id: String,
    note_date: String,
    title: String,
    body_text: String,
    updated_at_ms: i64,
    revision: i64,
    is_pinned: i64,
}

impl SearchRow {
    fn from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            id: row.get(0)?,
            note_date: row.get(1)?,
            title: row.get(2)?,
            body_text: row.get(3)?,
            updated_at_ms: row.get(4)?,
            revision: row.get(5)?,
            is_pinned: row.get(6)?,
        })
    }

    fn into_hit(
        self,
        query: &str,
        matching_tags: Vec<SearchMatchTag>,
    ) -> Result<SearchHit, FoundationError> {
        validate_note_id(&self.id)?;
        validate_note_date(&self.note_date)?;
        let revision = u64::try_from(self.revision)
            .ok()
            .filter(|revision| *revision > 0)
            .ok_or_else(FoundationError::validation_failed)?;
        let is_pinned = match self.is_pinned {
            0 => false,
            1 => true,
            _ => return Err(FoundationError::validation_failed()),
        };
        if self.updated_at_ms < 0 {
            return Err(FoundationError::validation_failed());
        }
        Ok(SearchHit {
            note: NoteSummary {
                id: self.id,
                note_date: self.note_date,
                title: self.title,
                updated_at_ms: self.updated_at_ms,
                revision,
                is_pinned,
            },
            snippet: search_snippet(&self.body_text, query),
            matching_tags,
        })
    }
}

fn escaped_like_pattern(query: &str) -> String {
    format!(
        "%{}%",
        query
            .replace('\\', "\\\\")
            .replace('%', "\\%")
            .replace('_', "\\_")
    )
}

fn search_snippet(body_text: &str, query: &str) -> String {
    const MAX_CHARS: usize = 140;
    const CONTEXT_BEFORE: usize = 40;
    let match_byte = body_text.find(query).or_else(|| {
        query.is_ascii().then(|| {
            body_text
                .to_ascii_lowercase()
                .find(&query.to_ascii_lowercase())
        })?
    });
    let match_char = match_byte.map(|byte| body_text[..byte].chars().count());
    let body = body_text.chars().collect::<Vec<_>>();
    if body.is_empty() {
        return String::new();
    }
    let start = match_char
        .map(|index| index.saturating_sub(CONTEXT_BEFORE))
        .unwrap_or_default();
    let end = (start + MAX_CHARS).min(body.len());
    let mut snippet = body[start..end].iter().collect::<String>();
    if start > 0 {
        snippet.insert(0, '…');
    }
    if end < body.len() {
        snippet.push('…');
    }
    snippet
}

impl SqlCipherStore {
    fn with_connection<T>(
        &self,
        action: impl FnOnce(&mut Connection) -> Result<T, FoundationError>,
    ) -> Result<T, FoundationError> {
        let mut open = self.open.lock().map_err(|_| repository_unavailable())?;
        let connection = open
            .as_mut()
            .map(|database| &mut database.connection)
            .ok_or_else(repository_unavailable)?;
        action(connection)
    }
}

struct StoredAssetRow {
    asset_id: String,
    media_type: String,
    source_format: String,
    storage_relpath: String,
    byte_size_plain: i64,
    byte_size_cipher: i64,
    pixel_width: i64,
    pixel_height: i64,
    sha256_plain: Vec<u8>,
    crypto_format_version: i64,
    key_id: String,
    created_at_ms: i64,
}

impl StoredAssetRow {
    fn from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            asset_id: row.get(0)?,
            media_type: row.get(1)?,
            source_format: row.get(2)?,
            storage_relpath: row.get(3)?,
            byte_size_plain: row.get(4)?,
            byte_size_cipher: row.get(5)?,
            pixel_width: row.get(6)?,
            pixel_height: row.get(7)?,
            sha256_plain: row.get(8)?,
            crypto_format_version: row.get(9)?,
            key_id: row.get(10)?,
            created_at_ms: row.get(11)?,
        })
    }

    fn into_record(self) -> Result<AssetRecord, FoundationError> {
        let source_format = match self.source_format.as_str() {
            "png" => ImageSourceFormat::Png,
            "jpeg" => ImageSourceFormat::Jpeg,
            "webp" => ImageSourceFormat::WebP,
            _ => return Err(database_corrupted()),
        };
        let record = AssetRecord {
            asset_id: self.asset_id,
            media_type: self.media_type,
            source_format,
            storage_relpath: self.storage_relpath,
            byte_size_plain: self
                .byte_size_plain
                .try_into()
                .map_err(|_| database_corrupted())?,
            byte_size_cipher: self
                .byte_size_cipher
                .try_into()
                .map_err(|_| database_corrupted())?,
            pixel_width: self
                .pixel_width
                .try_into()
                .map_err(|_| database_corrupted())?,
            pixel_height: self
                .pixel_height
                .try_into()
                .map_err(|_| database_corrupted())?,
            sha256_plain: self
                .sha256_plain
                .try_into()
                .map_err(|_| database_corrupted())?,
            crypto_format_version: self
                .crypto_format_version
                .try_into()
                .map_err(|_| database_corrupted())?,
            key_id: self.key_id,
            created_at_ms: self.created_at_ms,
        };
        record.validate().map_err(|_| database_corrupted())?;
        Ok(record)
    }
}

fn read_asset(
    connection: &Connection,
    asset_id: &str,
) -> Result<Option<AssetRecord>, FoundationError> {
    connection
        .query_row(
            "SELECT id, media_type, source_format, storage_relpath, byte_size_plain, \
                    byte_size_cipher, pixel_width, pixel_height, sha256_plain, \
                    crypto_format_version, key_id, created_at_ms \
               FROM assets WHERE id = ?1",
            [asset_id],
            StoredAssetRow::from_row,
        )
        .optional()
        .map_err(map_repository_sqlite)?
        .map(StoredAssetRow::into_record)
        .transpose()
}

struct StoredNoteRow {
    id: String,
    note_date: String,
    title: String,
    body_json: String,
    body_format: String,
    body_schema_version: i64,
    body_text: String,
    content_hash: Vec<u8>,
    created_at_ms: i64,
    updated_at_ms: i64,
    revision: i64,
    is_pinned: i64,
}

impl StoredNoteRow {
    fn from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            id: row.get(0)?,
            note_date: row.get(1)?,
            title: row.get(2)?,
            body_json: row.get(3)?,
            body_format: row.get(4)?,
            body_schema_version: row.get(5)?,
            body_text: row.get(6)?,
            content_hash: row.get(7)?,
            created_at_ms: row.get(8)?,
            updated_at_ms: row.get(9)?,
            revision: row.get(10)?,
            is_pinned: row.get(11)?,
        })
    }

    fn into_note(self) -> Result<Note, FoundationError> {
        let is_pinned = match self.is_pinned {
            0 => false,
            1 => true,
            _ => return Err(database_corrupted()),
        };
        let note = Note {
            id: self.id,
            note_date: self.note_date,
            title: self.title,
            body_json: self.body_json,
            body_format: self.body_format,
            body_schema_version: self
                .body_schema_version
                .try_into()
                .map_err(|_| database_corrupted())?,
            body_text: self.body_text,
            content_hash: self
                .content_hash
                .try_into()
                .map_err(|_| database_corrupted())?,
            created_at_ms: self.created_at_ms,
            updated_at_ms: self.updated_at_ms,
            revision: self.revision.try_into().map_err(|_| database_corrupted())?,
            is_pinned,
        };
        note.validate_stored().map_err(|_| database_corrupted())?;
        Ok(note)
    }
}

fn read_note(connection: &Connection, id: &str) -> Result<Option<Note>, FoundationError> {
    connection
        .query_row(
            "SELECT id, note_date, title, body_json, body_format, body_schema_version, \
                    body_text, content_hash, created_at_ms, updated_at_ms, revision, is_pinned \
               FROM notes WHERE id = ?1 AND deleted_at_ms IS NULL",
            [id],
            StoredNoteRow::from_row,
        )
        .optional()
        .map_err(map_repository_sqlite)?
        .map(StoredNoteRow::into_note)
        .transpose()
}

struct StoredTagRow {
    id: String,
    name: String,
    normalized_name: String,
    is_seed_default: i64,
    created_at_ms: i64,
    updated_at_ms: i64,
}

impl StoredTagRow {
    fn from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            id: row.get(0)?,
            name: row.get(1)?,
            normalized_name: row.get(2)?,
            is_seed_default: row.get(3)?,
            created_at_ms: row.get(4)?,
            updated_at_ms: row.get(5)?,
        })
    }

    fn into_tag(self) -> Result<Tag, FoundationError> {
        let is_seed_default = match self.is_seed_default {
            0 => false,
            1 => true,
            _ => return Err(database_corrupted()),
        };
        let tag = Tag {
            id: self.id,
            name: self.name,
            normalized_name: self.normalized_name,
            is_seed_default,
            created_at_ms: self.created_at_ms,
            updated_at_ms: self.updated_at_ms,
        };
        tag.validate_stored().map_err(|_| database_corrupted())?;
        Ok(tag)
    }
}

fn read_tag(connection: &Connection, id: &str) -> Result<Option<Tag>, FoundationError> {
    connection
        .query_row(
            "SELECT id, name, normalized_name, is_seed_default, created_at_ms, updated_at_ms \
               FROM tags WHERE id = ?1",
            [id],
            StoredTagRow::from_row,
        )
        .optional()
        .map_err(map_repository_sqlite)?
        .map(StoredTagRow::into_tag)
        .transpose()
}

fn read_tag_by_normalized_name(
    connection: &Connection,
    normalized_name: &str,
) -> Result<Option<Tag>, FoundationError> {
    connection
        .query_row(
            "SELECT id, name, normalized_name, is_seed_default, created_at_ms, updated_at_ms \
               FROM tags WHERE normalized_name = ?1",
            [normalized_name],
            StoredTagRow::from_row,
        )
        .optional()
        .map_err(map_repository_sqlite)?
        .map(StoredTagRow::into_tag)
        .transpose()
}

fn read_tags_for_note(connection: &Connection, note_id: &str) -> Result<Vec<Tag>, FoundationError> {
    let mut statement = connection
        .prepare(
            "SELECT t.id, t.name, t.normalized_name, t.is_seed_default, \
                    t.created_at_ms, t.updated_at_ms \
               FROM tags t JOIN note_tags nt ON nt.tag_id = t.id \
              WHERE nt.note_id = ?1 \
              ORDER BY t.normalized_name ASC, t.id ASC",
        )
        .map_err(map_repository_sqlite)?;
    statement
        .query_map([note_id], StoredTagRow::from_row)
        .map_err(map_repository_sqlite)?
        .map(|row| row.map_err(map_repository_sqlite)?.into_tag())
        .collect()
}

fn mutate_note_tag(
    connection: &mut Connection,
    input: NoteTagUpdate,
    assign: bool,
) -> Result<Note, FoundationError> {
    let base_revision =
        i64::try_from(input.base_revision).map_err(|_| FoundationError::validation_failed())?;
    let transaction = connection.transaction().map_err(map_repository_sqlite)?;
    let current =
        read_note(&transaction, &input.note_id)?.ok_or_else(FoundationError::note_not_found)?;
    if current.revision != input.base_revision {
        return Err(FoundationError::revision_conflict());
    }
    if read_tag(&transaction, &input.tag_id)?.is_none() {
        return Err(FoundationError::tag_not_found());
    }

    let changed = if assign {
        transaction
            .execute(
                "INSERT OR IGNORE INTO note_tags(note_id, tag_id, created_at_ms) VALUES (?1, ?2, ?3)",
                params![input.note_id, input.tag_id, input.updated_at_ms],
            )
            .map_err(map_repository_sqlite)?
    } else {
        transaction
            .execute(
                "DELETE FROM note_tags WHERE note_id = ?1 AND tag_id = ?2",
                params![input.note_id, input.tag_id],
            )
            .map_err(map_repository_sqlite)?
    };

    if changed != 0 {
        let affected = transaction
            .execute(
                "UPDATE notes SET updated_at_ms = ?1, revision = revision + 1 \
                  WHERE id = ?2 AND revision = ?3",
                params![input.updated_at_ms, input.note_id, base_revision],
            )
            .map_err(map_repository_sqlite)?;
        if affected != 1 {
            return Err(FoundationError::revision_conflict());
        }
    }
    let updated =
        read_note(&transaction, &input.note_id)?.ok_or_else(FoundationError::note_not_found)?;
    transaction.commit().map_err(map_repository_sqlite)?;
    Ok(updated)
}

fn map_tag_sqlite(error: rusqlite::Error) -> FoundationError {
    if let rusqlite::Error::SqliteFailure(raw, _) = &error
        && raw.extended_code & 0xff == rusqlite::ffi::SQLITE_CONSTRAINT
    {
        return FoundationError::tag_name_conflict();
    }
    map_repository_sqlite(error)
}

fn map_repository_sqlite(error: rusqlite::Error) -> FoundationError {
    if let rusqlite::Error::SqliteFailure(raw, _) = &error {
        if raw.extended_code & 0xff == rusqlite::ffi::SQLITE_FULL {
            return FoundationError::new(ErrorCode::DiskFull, "The disk is full.");
        }
        if raw.extended_code & 0xff == rusqlite::ffi::SQLITE_CONSTRAINT {
            return FoundationError::validation_failed();
        }
    }
    repository_unavailable()
}

fn repository_unavailable() -> FoundationError {
    FoundationError::new(
        ErrorCode::InternalError,
        "The encrypted Note store could not complete the operation.",
    )
}

fn database_corrupted() -> FoundationError {
    FoundationError::new(
        ErrorCode::DatabaseCorrupted,
        "The encrypted database contains an invalid Note record.",
    )
}

fn open_keyed(path: &Path, key: &SecretKey, create: bool) -> Result<Connection, FoundationError> {
    let mut flags = OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX;
    if create {
        flags |= OpenFlags::SQLITE_OPEN_CREATE;
    }
    let connection = Connection::open_with_flags(path, flags)
        .map_err(|error| map_sqlite(error, ErrorCode::DataRootUnavailable))?;
    connection
        .busy_timeout(Duration::from_secs(5))
        .map_err(|error| map_sqlite(error, ErrorCode::DataRootUnavailable))?;
    apply_raw_key(&connection, key)?;

    connection
        .query_row("SELECT count(*) FROM sqlite_master", [], |row| {
            row.get::<_, i64>(0)
        })
        .map_err(|error| {
            map_sqlite(
                error,
                if create {
                    ErrorCode::DatabaseCorrupted
                } else {
                    ErrorCode::DatabaseWrongKey
                },
            )
        })?;
    verify_binary_identity(&connection)?;
    verify_cipher_integrity(&connection)?;
    Ok(connection)
}

fn apply_raw_key(connection: &Connection, key: &SecretKey) -> Result<(), FoundationError> {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut sql = Zeroizing::new(String::with_capacity(22 + key.as_bytes().len() * 2));
    sql.push_str("PRAGMA key = \"x'");
    for byte in key.as_bytes() {
        sql.push(DIGITS[(byte >> 4) as usize] as char);
        sql.push(DIGITS[(byte & 0x0f) as usize] as char);
    }
    sql.push_str("'\";");
    connection
        .execute_batch(sql.as_str())
        .map_err(|error| map_sqlite(error, ErrorCode::DatabaseWrongKey))
}

fn verify_binary_identity(connection: &Connection) -> Result<(), FoundationError> {
    let cipher = cipher_version(connection)?;
    let sqlite: String = connection
        .query_row("SELECT sqlite_version()", [], |row| row.get(0))
        .map_err(|error| map_sqlite(error, ErrorCode::InternalError))?;
    if cipher != EXPECTED_CIPHER_VERSION || sqlite != EXPECTED_SQLITE_VERSION {
        return Err(FoundationError::new(
            ErrorCode::InternalError,
            "The encrypted database runtime does not match the frozen production dependency baseline.",
        ));
    }

    let mut statement = connection
        .prepare("PRAGMA compile_options")
        .map_err(|error| map_sqlite(error, ErrorCode::InternalError))?;
    let options = statement
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(|error| map_sqlite(error, ErrorCode::InternalError))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| map_sqlite(error, ErrorCode::InternalError))?;
    for required in ["HAS_CODEC", "ENABLE_FTS5", "TEMP_STORE=3"] {
        if !options.iter().any(|option| option == required) {
            let message = match required {
                "HAS_CODEC" => "The encrypted database runtime is missing HAS_CODEC.",
                "ENABLE_FTS5" => "The encrypted database runtime is missing ENABLE_FTS5.",
                "TEMP_STORE=3" => "The encrypted database runtime is missing TEMP_STORE=3.",
                _ => "The encrypted database runtime is missing a required compile option.",
            };
            return Err(FoundationError::new(ErrorCode::InternalError, message));
        }
    }
    Ok(())
}

fn verify_cipher_integrity(connection: &Connection) -> Result<(), FoundationError> {
    let mut statement = connection
        .prepare("PRAGMA cipher_integrity_check")
        .map_err(|error| map_sqlite(error, ErrorCode::DatabaseCorrupted))?;
    let messages = statement
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(|error| map_sqlite(error, ErrorCode::DatabaseCorrupted))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| map_sqlite(error, ErrorCode::DatabaseCorrupted))?;
    if messages
        .iter()
        .any(|message| !message.eq_ignore_ascii_case("ok"))
    {
        return Err(FoundationError::new(
            ErrorCode::DatabaseCorrupted,
            "The encrypted database failed its integrity check.",
        ));
    }
    Ok(())
}

fn configure_runtime(connection: &Connection) -> Result<(), FoundationError> {
    connection
        .execute_batch(
            "PRAGMA foreign_keys=ON;\
             PRAGMA temp_store=MEMORY;\
             PRAGMA wal_autocheckpoint=1000;",
        )
        .map_err(|error| map_sqlite(error, ErrorCode::DataRootUnavailable))?;
    let journal_mode: String = connection
        .query_row("PRAGMA journal_mode=WAL", [], |row| row.get(0))
        .map_err(|error| map_sqlite(error, ErrorCode::DataRootUnavailable))?;
    let foreign_keys: i64 = connection
        .query_row("PRAGMA foreign_keys", [], |row| row.get(0))
        .map_err(|error| map_sqlite(error, ErrorCode::InternalError))?;
    let temp_store: i64 = connection
        .query_row("PRAGMA temp_store", [], |row| row.get(0))
        .map_err(|error| map_sqlite(error, ErrorCode::InternalError))?;
    if !journal_mode.eq_ignore_ascii_case("wal") || foreign_keys != 1 || temp_store != 2 {
        return Err(FoundationError::new(
            ErrorCode::InternalError,
            "The encrypted database runtime safety settings could not be applied.",
        ));
    }
    Ok(())
}

fn migrate(
    connection: &mut Connection,
    registry: &MigrationRegistry,
    app_version: &str,
) -> Result<(), FoundationError> {
    let current = schema_version(connection)?;
    if current > registry.latest_version() {
        return Err(FoundationError::new(
            ErrorCode::DatabaseSchemaTooNew,
            "This encrypted database was created by a newer Desktop Notes version.",
        ));
    }

    if current > 0 {
        verify_applied_migrations(connection, registry, current)?;
    }
    for migration in registry
        .migrations()
        .iter()
        .filter(|migration| migration.version() > current)
    {
        let transaction = connection.transaction().map_err(|_| migration_failed())?;
        transaction
            .execute_batch(migration.sql())
            .map_err(|_| migration_failed())?;
        transaction
            .execute(
                "INSERT INTO schema_migrations(version, name, checksum, applied_at_ms, app_version)\
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    migration.version(),
                    migration.name(),
                    migration.checksum(),
                    unix_time_ms()?,
                    app_version
                ],
            )
            .map_err(|_| migration_failed())?;
        transaction
            .pragma_update(None, "user_version", migration.version())
            .map_err(|_| migration_failed())?;
        transaction.commit().map_err(|_| migration_failed())?;
    }
    verify_applied_migrations(connection, registry, registry.latest_version())
}

fn verify_applied_migrations(
    connection: &Connection,
    registry: &MigrationRegistry,
    through_version: u32,
) -> Result<(), FoundationError> {
    for migration in registry
        .migrations()
        .iter()
        .filter(|migration| migration.version() <= through_version)
    {
        let applied: Option<(String, String)> = connection
            .query_row(
                "SELECT name, checksum FROM schema_migrations WHERE version = ?1",
                [migration.version()],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .map_err(|_| migration_failed())?;
        match applied {
            Some((name, checksum))
                if name == migration.name() && checksum == migration.checksum() => {}
            _ => return Err(migration_failed()),
        }
    }
    Ok(())
}

fn schema_version(connection: &Connection) -> Result<u32, FoundationError> {
    connection
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .map_err(|error| map_sqlite(error, ErrorCode::DatabaseCorrupted))
}

fn cipher_version(connection: &Connection) -> Result<String, FoundationError> {
    connection
        .query_row("PRAGMA cipher_version", [], |row| row.get(0))
        .map_err(|error| map_sqlite(error, ErrorCode::InternalError))
}

fn unix_time_ms() -> Result<i64, FoundationError> {
    let millis = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_err(|_| migration_failed())?
        .as_millis();
    i64::try_from(millis).map_err(|_| migration_failed())
}

fn staging_path(parent: &Path) -> Result<PathBuf, FoundationError> {
    let mut suffix = [0_u8; 12];
    getrandom::fill(&mut suffix).map_err(|_| {
        FoundationError::new(
            ErrorCode::InternalError,
            "A secure initialization identifier could not be generated.",
        )
    })?;
    Ok(parent.join(format!(".desktop-notes-{}.init", hex(&suffix))))
}

fn remove_staging_files(staging: &Path) {
    for path in [
        staging.to_path_buf(),
        PathBuf::from(format!("{}-wal", staging.display())),
        PathBuf::from(format!("{}-shm", staging.display())),
    ] {
        let _ = fs::remove_file(path);
    }
}

fn map_io(error: io::Error) -> FoundationError {
    if error.raw_os_error() == Some(112) {
        FoundationError::new(ErrorCode::DiskFull, "The disk is full.")
    } else {
        data_root_unavailable()
    }
}

fn map_sqlite(error: rusqlite::Error, default: ErrorCode) -> FoundationError {
    if let rusqlite::Error::SqliteFailure(raw, _) = &error
        && raw.extended_code & 0xff == rusqlite::ffi::SQLITE_FULL
    {
        return FoundationError::new(ErrorCode::DiskFull, "The disk is full.");
    }
    let message = match default {
        ErrorCode::DatabaseWrongKey => {
            "The encrypted database key was rejected. Existing data was not replaced."
        }
        ErrorCode::DatabaseCorrupted => "The encrypted database appears to be corrupted.",
        ErrorCode::DataRootUnavailable => "The local data directory is unavailable.",
        _ => "The encrypted database runtime failed.",
    };
    FoundationError::new(default, message)
}

fn data_root_unavailable() -> FoundationError {
    FoundationError::new(
        ErrorCode::DataRootUnavailable,
        "The local data directory is unavailable.",
    )
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

fn decode_hex_32(value: &str) -> Result<[u8; 32], FoundationError> {
    if value.len() != 64 {
        return Err(FoundationError::backup_failed());
    }
    let mut output = [0_u8; 32];
    for (index, pair) in value.as_bytes().as_chunks::<2>().0.iter().enumerate() {
        let high = hex_nibble(pair[0]).ok_or_else(FoundationError::backup_failed)?;
        let low = hex_nibble(pair[1]).ok_or_else(FoundationError::backup_failed)?;
        output[index] = (high << 4) | low;
    }
    Ok(output)
}

fn hex_nibble(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn window_preferences_reject_invalid_or_oversized_json_before_storage_access() {
        let store = SqlCipherStore::new(PathBuf::from("not-opened.db"), "test");
        assert_eq!(
            store
                .save_window_preferences("not-json", 1)
                .unwrap_err()
                .code(),
            ErrorCode::ValidationFailed
        );
        let oversized = format!("\"{}\"", "x".repeat(33 * 1024));
        assert_eq!(
            store
                .save_window_preferences(&oversized, 1)
                .unwrap_err()
                .code(),
            ErrorCode::ValidationFailed
        );
    }

    #[test]
    fn exclusive_copy_fallback_commits_closed_staging_database() {
        let root = std::env::temp_dir().join(format!(
            "desktop-notes-database-commit-{}",
            std::process::id()
        ));
        fs::create_dir_all(&root).unwrap();
        let staging = root.join("database.init");
        let destination = root.join("desktop-notes.db");
        let encrypted_fixture = b"synthetic-encrypted-database";
        fs::write(&staging, encrypted_fixture).unwrap();

        commit_staging_database_with(&staging, &destination, |_, _| {
            Err(io::Error::from(io::ErrorKind::CrossesDevices))
        })
        .unwrap();

        assert_eq!(fs::read(&destination).unwrap(), encrypted_fixture);
        assert!(!staging.exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn failed_fallback_copy_removes_exclusively_created_destination() {
        let root = std::env::temp_dir().join(format!(
            "desktop-notes-database-partial-{}",
            std::process::id()
        ));
        fs::create_dir_all(&root).unwrap();
        let staging = root.join("database.init");
        let destination = root.join("desktop-notes.db");
        fs::write(&staging, b"complete-encrypted-database").unwrap();

        let error =
            persist_staging_database_with(&staging, &destination, |source, destination_file| {
                let mut prefix = [0_u8; 4];
                use std::io::{Read as _, Write as _};
                source.read_exact(&mut prefix)?;
                destination_file.write_all(&prefix)?;
                Err(io::Error::new(io::ErrorKind::WriteZero, "injected"))
            })
            .unwrap_err();

        assert_eq!(error.code(), ErrorCode::DataRootUnavailable);
        assert!(!destination.exists());
        assert!(staging.exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn fallback_never_replaces_or_removes_an_existing_database() {
        let root = std::env::temp_dir().join(format!(
            "desktop-notes-database-existing-{}",
            std::process::id()
        ));
        fs::create_dir_all(&root).unwrap();
        let staging = root.join("database.init");
        let destination = root.join("desktop-notes.db");
        fs::write(&staging, b"new-encrypted-database").unwrap();
        fs::write(&destination, b"existing-encrypted-database").unwrap();

        let error = commit_staging_database_with(&staging, &destination, |_, _| {
            Err(io::Error::from(io::ErrorKind::CrossesDevices))
        })
        .unwrap_err();

        assert_eq!(error.code(), ErrorCode::DataRootUnavailable);
        assert_eq!(
            fs::read(&destination).unwrap(),
            b"existing-encrypted-database"
        );
        assert!(staging.exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn failed_fallback_sync_removes_exclusively_created_database() {
        let root = std::env::temp_dir().join(format!(
            "desktop-notes-database-sync-{}",
            std::process::id()
        ));
        fs::create_dir_all(&root).unwrap();
        let destination = root.join("desktop-notes.db");
        let mut exclusive = ExclusiveDestination::create(&destination).unwrap();
        use std::io::Write as _;
        exclusive.file_mut().write_all(b"encrypted").unwrap();

        let error = exclusive
            .sync_close_and_verify_length_with(
                9,
                9,
                |_| Err(io::Error::other("injected sync failure")),
                |path| fs::metadata(path).map(|metadata| metadata.len()),
            )
            .unwrap_err();

        assert_eq!(error.code(), ErrorCode::DataRootUnavailable);
        assert!(!destination.exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn failed_fallback_readback_removes_exclusively_created_database() {
        let root = std::env::temp_dir().join(format!(
            "desktop-notes-database-readback-{}",
            std::process::id()
        ));
        fs::create_dir_all(&root).unwrap();
        let destination = root.join("desktop-notes.db");
        let mut exclusive = ExclusiveDestination::create(&destination).unwrap();
        use std::io::Write as _;
        exclusive.file_mut().write_all(b"encrypted").unwrap();

        let error = exclusive
            .sync_close_and_verify_length_with(
                9,
                9,
                |file| file.sync_all(),
                |_| Err(io::Error::other("injected readback failure")),
            )
            .unwrap_err();

        assert_eq!(error.code(), ErrorCode::DataRootUnavailable);
        assert!(!destination.exists());
        fs::remove_dir_all(root).unwrap();
    }
}
