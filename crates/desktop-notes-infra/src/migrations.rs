use desktop_notes_core::{ErrorCode, FoundationError};
use sha2::{Digest, Sha256};

const B01_FOUNDATION_SQL: &str = r#"
CREATE TABLE schema_migrations (
  version            INTEGER PRIMARY KEY,
  name               TEXT NOT NULL,
  checksum           TEXT NOT NULL,
  applied_at_ms      INTEGER NOT NULL,
  app_version        TEXT NOT NULL
);

CREATE TABLE app_meta (
  key                TEXT PRIMARY KEY,
  value_json         TEXT NOT NULL,
  updated_at_ms      INTEGER NOT NULL
);
"#;

const B02_NOTES_SQL: &str = r#"
CREATE TABLE notes (
  row_id              INTEGER PRIMARY KEY,
  id                  TEXT NOT NULL UNIQUE,
  note_date           TEXT NOT NULL,
  title               TEXT NOT NULL DEFAULT '',
  body_json           TEXT NOT NULL,
  body_format         TEXT NOT NULL DEFAULT 'tiptap-json',
  body_schema_version INTEGER NOT NULL,
  body_text           TEXT NOT NULL DEFAULT '',
  content_hash        BLOB NOT NULL,
  created_at_ms       INTEGER NOT NULL,
  updated_at_ms       INTEGER NOT NULL,
  revision            INTEGER NOT NULL DEFAULT 1,
  is_pinned           INTEGER NOT NULL DEFAULT 0 CHECK (is_pinned IN (0,1)),
  is_favorite         INTEGER NOT NULL DEFAULT 0 CHECK (is_favorite IN (0,1)),
  archived_at_ms      INTEGER NULL,
  deleted_at_ms       INTEGER NULL,
  CHECK (length(id) = 36),
  CHECK (length(note_date) = 10),
  CHECK (length(title) <= 500),
  CHECK (length(body_json) <= 1048576),
  CHECK (json_valid(body_json)),
  CHECK (body_format = 'tiptap-json'),
  CHECK (body_schema_version = 1),
  CHECK (length(content_hash) = 32),
  CHECK (created_at_ms >= 0),
  CHECK (updated_at_ms >= created_at_ms),
  CHECK (revision >= 1)
);

CREATE INDEX idx_notes_date_active
  ON notes(note_date, is_pinned DESC, updated_at_ms DESC)
  WHERE deleted_at_ms IS NULL AND archived_at_ms IS NULL;

CREATE INDEX idx_notes_recent_active
  ON notes(updated_at_ms DESC)
  WHERE deleted_at_ms IS NULL AND archived_at_ms IS NULL;

CREATE INDEX idx_notes_favorites
  ON notes(updated_at_ms DESC)
  WHERE is_favorite = 1 AND deleted_at_ms IS NULL;

CREATE INDEX idx_notes_archive
  ON notes(archived_at_ms DESC)
  WHERE archived_at_ms IS NOT NULL AND deleted_at_ms IS NULL;

CREATE INDEX idx_notes_recycle
  ON notes(deleted_at_ms)
  WHERE deleted_at_ms IS NOT NULL;
"#;

const B05_ORGANIZATION_SQL: &str = r#"
CREATE TABLE tags (
  id                 TEXT PRIMARY KEY,
  name               TEXT NOT NULL,
  normalized_name    TEXT NOT NULL UNIQUE,
  is_seed_default    INTEGER NOT NULL DEFAULT 0 CHECK (is_seed_default IN (0,1)),
  color_token        TEXT NULL,
  created_at_ms      INTEGER NOT NULL,
  updated_at_ms      INTEGER NOT NULL,
  CHECK (length(id) = 36),
  CHECK (length(name) BETWEEN 1 AND 64),
  CHECK (length(CAST(name AS BLOB)) <= 256),
  CHECK (length(normalized_name) BETWEEN 1 AND 64),
  CHECK (length(CAST(normalized_name AS BLOB)) <= 256),
  CHECK (created_at_ms >= 0),
  CHECK (updated_at_ms >= created_at_ms)
);

CREATE TABLE note_tags (
  note_id            TEXT NOT NULL REFERENCES notes(id) ON DELETE CASCADE,
  tag_id             TEXT NOT NULL REFERENCES tags(id) ON DELETE CASCADE,
  created_at_ms      INTEGER NOT NULL CHECK (created_at_ms >= 0),
  PRIMARY KEY (note_id, tag_id)
);

CREATE INDEX idx_note_tags_tag ON note_tags(tag_id, note_id);

INSERT INTO tags(id, name, normalized_name, is_seed_default, created_at_ms, updated_at_ms) VALUES
  ('00000000-0000-4000-8000-000000000101', 'Work',     'work',     1, 0, 0),
  ('00000000-0000-4000-8000-000000000102', 'Personal', 'personal', 1, 0, 0);
"#;

const B06_SEARCH_SQL: &str = r#"
CREATE VIRTUAL TABLE note_fts USING fts5(note_id UNINDEXED, title, body_text, tags_text, tokenize='trigram');
INSERT INTO note_fts(note_id, title, body_text, tags_text)
SELECT n.id, n.title, n.body_text, COALESCE((SELECT group_concat(t.name, ' ') FROM note_tags nt JOIN tags t ON t.id = nt.tag_id WHERE nt.note_id = n.id), '') FROM notes n WHERE n.deleted_at_ms IS NULL;
CREATE TRIGGER notes_fts_ai AFTER INSERT ON notes BEGIN INSERT INTO note_fts(note_id,title,body_text,tags_text) SELECT new.id,new.title,new.body_text,COALESCE((SELECT group_concat(t.name,' ') FROM note_tags nt JOIN tags t ON t.id=nt.tag_id WHERE nt.note_id=new.id),'') WHERE new.deleted_at_ms IS NULL; END;
CREATE TRIGGER notes_fts_au AFTER UPDATE OF title,body_text,deleted_at_ms ON notes BEGIN DELETE FROM note_fts WHERE note_id=old.id; INSERT INTO note_fts(note_id,title,body_text,tags_text) SELECT new.id,new.title,new.body_text,COALESCE((SELECT group_concat(t.name,' ') FROM note_tags nt JOIN tags t ON t.id=nt.tag_id WHERE nt.note_id=new.id),'') WHERE new.deleted_at_ms IS NULL; END;
CREATE TRIGGER notes_fts_ad AFTER DELETE ON notes BEGIN DELETE FROM note_fts WHERE note_id=old.id; END;
CREATE TRIGGER note_tags_fts_ai AFTER INSERT ON note_tags BEGIN DELETE FROM note_fts WHERE note_id=new.note_id; INSERT INTO note_fts(note_id,title,body_text,tags_text) SELECT n.id,n.title,n.body_text,COALESCE((SELECT group_concat(t.name,' ') FROM note_tags nt JOIN tags t ON t.id=nt.tag_id WHERE nt.note_id=n.id),'') FROM notes n WHERE n.id=new.note_id AND n.deleted_at_ms IS NULL; END;
CREATE TRIGGER note_tags_fts_ad AFTER DELETE ON note_tags BEGIN DELETE FROM note_fts WHERE note_id=old.note_id; INSERT INTO note_fts(note_id,title,body_text,tags_text) SELECT n.id,n.title,n.body_text,COALESCE((SELECT group_concat(t.name,' ') FROM note_tags nt JOIN tags t ON t.id=nt.tag_id WHERE nt.note_id=n.id),'') FROM notes n WHERE n.id=old.note_id AND n.deleted_at_ms IS NULL; END;
CREATE TRIGGER tags_fts_au AFTER UPDATE OF name ON tags BEGIN DELETE FROM note_fts WHERE note_id IN (SELECT note_id FROM note_tags WHERE tag_id=new.id); INSERT INTO note_fts(note_id,title,body_text,tags_text) SELECT n.id,n.title,n.body_text,COALESCE((SELECT group_concat(t.name,' ') FROM note_tags nt JOIN tags t ON t.id=nt.tag_id WHERE nt.note_id=n.id),'') FROM notes n WHERE n.id IN (SELECT note_id FROM note_tags WHERE tag_id=new.id) AND n.deleted_at_ms IS NULL; END;
"#;

const B07_ENCRYPTED_IMAGES_SQL: &str = r#"
CREATE TABLE assets (
  id                    TEXT PRIMARY KEY,
  media_type            TEXT NOT NULL CHECK (media_type = 'image/png'),
  source_format         TEXT NOT NULL CHECK (source_format IN ('png','jpeg','webp')),
  storage_relpath       TEXT NOT NULL UNIQUE,
  byte_size_plain       INTEGER NOT NULL CHECK (byte_size_plain > 0),
  byte_size_cipher      INTEGER NOT NULL CHECK (byte_size_cipher > 0),
  pixel_width           INTEGER NOT NULL CHECK (pixel_width > 0),
  pixel_height          INTEGER NOT NULL CHECK (pixel_height > 0),
  sha256_plain          BLOB NOT NULL CHECK (length(sha256_plain) = 32),
  crypto_format_version INTEGER NOT NULL CHECK (crypto_format_version = 1),
  key_id                TEXT NOT NULL CHECK (key_id = 'assets-v1'),
  state                 TEXT NOT NULL CHECK (state IN ('ready','gc_pending')),
  created_at_ms         INTEGER NOT NULL CHECK (created_at_ms >= 0)
);

CREATE TABLE note_assets (
  note_id               TEXT NOT NULL REFERENCES notes(id) ON DELETE CASCADE,
  asset_id              TEXT NOT NULL REFERENCES assets(id) ON DELETE RESTRICT,
  occurrence_id         TEXT NOT NULL,
  sort_order            INTEGER NOT NULL CHECK (sort_order >= 0),
  created_at_ms         INTEGER NOT NULL CHECK (created_at_ms >= 0),
  PRIMARY KEY (note_id, occurrence_id)
);

CREATE INDEX idx_note_assets_asset ON note_assets(asset_id);

CREATE TABLE asset_gc_queue (
  asset_id               TEXT PRIMARY KEY REFERENCES assets(id) ON DELETE CASCADE,
  not_before_ms          INTEGER NOT NULL CHECK (not_before_ms >= 0),
  attempts               INTEGER NOT NULL DEFAULT 0 CHECK (attempts >= 0),
  last_error_code        TEXT NULL
);
"#;

const B08_QUICK_CAPTURE_SQL: &str = r#"
CREATE TABLE user_settings (
  key           TEXT PRIMARY KEY,
  value_json    TEXT NOT NULL,
  updated_at_ms INTEGER NOT NULL CHECK (updated_at_ms >= 0)
);

INSERT INTO user_settings(key, value_json, updated_at_ms)
VALUES ('quick_capture_shortcut', '"Ctrl+Alt+N"', 0);
"#;

const B09_AUTOMATIC_BACKUP_SQL: &str = r#"
CREATE TABLE backup_catalog (
  id                    TEXT PRIMARY KEY,
  kind                  TEXT NOT NULL CHECK (kind = 'automatic'),
  storage_path          TEXT NOT NULL UNIQUE,
  local_day             TEXT NOT NULL,
  created_at_ms         INTEGER NOT NULL CHECK (created_at_ms >= 0),
  completed_at_ms       INTEGER NULL CHECK (completed_at_ms IS NULL OR completed_at_ms >= created_at_ms),
  status                TEXT NOT NULL CHECK (status IN ('valid','invalid','delete_pending')),
  format_version        INTEGER NOT NULL CHECK (format_version = 1),
  source_schema_version INTEGER NOT NULL,
  package_size          INTEGER NULL CHECK (package_size IS NULL OR package_size > 0),
  package_sha256        BLOB NULL CHECK (package_sha256 IS NULL OR length(package_sha256) = 32),
  protection_profile    TEXT NOT NULL CHECK (protection_profile = 'automatic-dpapi-current-user'),
  last_error_code       TEXT NULL
);

CREATE INDEX idx_backup_catalog_valid_created
  ON backup_catalog(created_at_ms DESC)
  WHERE status = 'valid';
"#;

#[derive(Clone, Debug)]
pub struct Migration {
    version: u32,
    name: String,
    sql: String,
    checksum: String,
}

impl Migration {
    pub fn new(version: u32, name: impl Into<String>, sql: impl Into<String>) -> Self {
        let sql = sql.into();
        let checksum = hex(&Sha256::digest(sql.as_bytes()));
        Self {
            version,
            name: name.into(),
            sql,
            checksum,
        }
    }

    pub(crate) fn version(&self) -> u32 {
        self.version
    }

    pub(crate) fn name(&self) -> &str {
        &self.name
    }

    pub(crate) fn sql(&self) -> &str {
        &self.sql
    }

    pub(crate) fn checksum(&self) -> &str {
        &self.checksum
    }
}

#[derive(Clone, Debug)]
pub struct MigrationRegistry {
    migrations: Vec<Migration>,
}

impl MigrationRegistry {
    pub fn b01() -> Self {
        Self {
            migrations: vec![Migration::new(
                1,
                "b01-secure-foundation",
                B01_FOUNDATION_SQL,
            )],
        }
    }

    pub fn b02() -> Self {
        Self::b01()
            .append(Migration::new(2, "b02-basic-note", B02_NOTES_SQL))
            .expect("the built-in B02 migration is sequential and named")
    }

    pub fn b05() -> Self {
        Self::b02()
            .append(Migration::new(
                3,
                "b05-tags-pin-recent",
                B05_ORGANIZATION_SQL,
            ))
            .expect("the built-in B05 migration is sequential and named")
    }

    pub fn b06() -> Self {
        Self::b05()
            .append(Migration::new(4, "b06-local-search", B06_SEARCH_SQL))
            .expect("built-in migration is sequential")
    }

    pub fn b07() -> Self {
        Self::b06()
            .append(Migration::new(
                5,
                "b07-encrypted-images",
                B07_ENCRYPTED_IMAGES_SQL,
            ))
            .expect("built-in migration is sequential")
    }

    pub fn b08() -> Self {
        Self::b07()
            .append(Migration::new(
                6,
                "b08-quick-capture-settings",
                B08_QUICK_CAPTURE_SQL,
            ))
            .expect("built-in migration is sequential")
    }

    pub fn b09() -> Self {
        Self::b08()
            .append(Migration::new(
                7,
                "b09-automatic-backup-catalog",
                B09_AUTOMATIC_BACKUP_SQL,
            ))
            .expect("built-in migration is sequential")
    }

    pub fn append(mut self, migration: Migration) -> Result<Self, FoundationError> {
        let expected = self.latest_version().saturating_add(1);
        if migration.version() != expected || migration.name().trim().is_empty() {
            return Err(migration_failed());
        }
        self.migrations.push(migration);
        Ok(self)
    }

    pub(crate) fn latest_version(&self) -> u32 {
        self.migrations
            .last()
            .map(Migration::version)
            .unwrap_or_default()
    }

    pub(crate) fn migrations(&self) -> &[Migration] {
        &self.migrations
    }
}

pub(crate) fn migration_failed() -> FoundationError {
    FoundationError::new(
        ErrorCode::MigrationFailed,
        "The encrypted database could not be migrated safely.",
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
