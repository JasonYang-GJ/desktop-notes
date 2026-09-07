use std::collections::HashSet;

use crate::{
    AssetFileStore, AssetIdGenerator, AssetStore, Clock, ErrorCode, FoundationError,
    validate_note_id,
};

pub const ASSET_GC_GRACE_MS: i64 = 24 * 60 * 60 * 1000;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ImageSourceFormat {
    Png,
    Jpeg,
    WebP,
}

impl ImageSourceFormat {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Png => "png",
            Self::Jpeg => "jpeg",
            Self::WebP => "webp",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ImageInputSource {
    EditorPaste,
    ClipboardScreenshot,
}

#[derive(Debug)]
pub struct ImageInput {
    pub source: ImageInputSource,
    pub declared_format: ImageSourceFormat,
    pub bytes: Vec<u8>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AssetRecord {
    pub asset_id: String,
    pub media_type: String,
    pub source_format: ImageSourceFormat,
    pub storage_relpath: String,
    pub byte_size_plain: u64,
    pub byte_size_cipher: u64,
    pub pixel_width: u32,
    pub pixel_height: u32,
    pub sha256_plain: [u8; 32],
    pub crypto_format_version: u16,
    pub key_id: String,
    pub created_at_ms: i64,
}

impl AssetRecord {
    pub fn validate(&self) -> Result<(), FoundationError> {
        validate_note_id(&self.asset_id)?;
        if self.media_type != "image/png"
            || self.storage_relpath.is_empty()
            || self.storage_relpath.contains('\\')
            || self.storage_relpath.starts_with('/')
            || self.storage_relpath.contains("..")
            || self.byte_size_plain == 0
            || self.byte_size_cipher == 0
            || self.pixel_width == 0
            || self.pixel_height == 0
            || self.crypto_format_version == 0
            || self.key_id != "assets-v1"
            || self.created_at_ms < 0
        {
            return Err(FoundationError::validation_failed());
        }
        Ok(())
    }
}

#[derive(Debug)]
pub struct PersistedImage {
    pub record: AssetRecord,
    pub normalized_png: Vec<u8>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StagedAssetImport {
    pub client_import_id: String,
    pub record: AssetRecord,
}

#[derive(Debug)]
pub struct ImportImageRequest {
    pub client_import_id: String,
    pub input: ImageInput,
}

#[derive(Debug)]
pub struct DisplayImage {
    pub asset_id: String,
    pub media_type: String,
    pub pixel_width: u32,
    pub pixel_height: u32,
    pub bytes: Vec<u8>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct AssetStartupRepair {
    pub staging_files_removed: u32,
    pub unknown_files_quarantined: u32,
    pub due_assets_removed: u32,
    pub due_assets_retained: u32,
}

pub struct ImageAssetService<'a> {
    repository: &'a dyn AssetStore,
    files: &'a dyn AssetFileStore,
    ids: &'a dyn AssetIdGenerator,
    clock: &'a dyn Clock,
}

impl<'a> ImageAssetService<'a> {
    pub fn new(
        repository: &'a dyn AssetStore,
        files: &'a dyn AssetFileStore,
        ids: &'a dyn AssetIdGenerator,
        clock: &'a dyn Clock,
    ) -> Self {
        Self {
            repository,
            files,
            ids,
            clock,
        }
    }

    pub fn import_image(
        &self,
        request: ImportImageRequest,
    ) -> Result<DisplayImage, FoundationError> {
        validate_note_id(&request.client_import_id)?;
        if let Some(existing) = self
            .repository
            .find_staged_import(&request.client_import_id)?
        {
            let bytes = self.files.read_image(
                &existing.record.asset_id,
                &existing.record.storage_relpath,
                &existing.record.sha256_plain,
                existing.record.pixel_width,
                existing.record.pixel_height,
            )?;
            return Ok(display_image(existing.record, bytes));
        }

        let asset_id = self.ids.new_asset_id()?;
        validate_note_id(&asset_id)?;
        let created_at_ms = self.clock.now_utc_ms()?;
        if created_at_ms < 0 {
            return Err(FoundationError::validation_failed());
        }
        let persisted = self.files.persist_image(&asset_id, request.input)?;
        let mut record = persisted.record;
        record.created_at_ms = created_at_ms;
        if let Err(error) = record.validate() {
            let _ = self
                .files
                .remove_image(&record.asset_id, &record.storage_relpath);
            return Err(error);
        }
        if let Err(error) = self.repository.stage_import(StagedAssetImport {
            client_import_id: request.client_import_id,
            record: record.clone(),
        }) {
            let _ = self
                .files
                .remove_image(&record.asset_id, &record.storage_relpath);
            return Err(error);
        }
        Ok(display_image(record, persisted.normalized_png))
    }

    pub fn read_image(
        &self,
        note_id: &str,
        asset_id: &str,
    ) -> Result<DisplayImage, FoundationError> {
        validate_note_id(note_id)?;
        validate_note_id(asset_id)?;
        let record = self
            .repository
            .get_ready_asset_for_note(note_id, asset_id)?
            .ok_or_else(FoundationError::asset_not_found)?;
        let bytes = self.files.read_image(
            &record.asset_id,
            &record.storage_relpath,
            &record.sha256_plain,
            record.pixel_width,
            record.pixel_height,
        )?;
        Ok(display_image(record, bytes))
    }

    pub fn discard_import(&self, client_import_id: &str) -> Result<bool, FoundationError> {
        validate_note_id(client_import_id)?;
        let Some(staged) = self.repository.discard_staged_import(client_import_id)? else {
            return Ok(false);
        };
        self.files
            .remove_image(&staged.record.asset_id, &staged.record.storage_relpath)?;
        Ok(true)
    }

    pub fn repair_at_startup(&self) -> Result<AssetStartupRepair, FoundationError> {
        let staging_files_removed = self.files.cleanup_staging()?;
        let tracked = self
            .repository
            .tracked_storage_relpaths()?
            .into_iter()
            .collect::<HashSet<_>>();
        let unknown_files_quarantined = self.files.quarantine_unknown_assets(&tracked)?;
        let now = self.clock.now_utc_ms()?;
        let mut repair = AssetStartupRepair {
            staging_files_removed,
            unknown_files_quarantined,
            ..AssetStartupRepair::default()
        };
        for record in self.repository.list_due_gc_assets(now, 64)? {
            match self
                .files
                .remove_image(&record.asset_id, &record.storage_relpath)
            {
                Ok(()) => {
                    if self.repository.finalize_gc_asset(&record.asset_id)? {
                        repair.due_assets_removed = repair.due_assets_removed.saturating_add(1);
                    } else {
                        repair.due_assets_retained = repair.due_assets_retained.saturating_add(1);
                    }
                }
                Err(error) => {
                    self.repository
                        .record_gc_failure(&record.asset_id, error.code())?;
                    repair.due_assets_retained = repair.due_assets_retained.saturating_add(1);
                }
            }
        }
        Ok(repair)
    }
}

fn display_image(record: AssetRecord, bytes: Vec<u8>) -> DisplayImage {
    DisplayImage {
        asset_id: record.asset_id,
        media_type: record.media_type,
        pixel_width: record.pixel_width,
        pixel_height: record.pixel_height,
        bytes,
    }
}

pub fn gc_error_code(error: ErrorCode) -> &'static str {
    match error {
        ErrorCode::DiskFull => "DISK_FULL",
        ErrorCode::DataRootUnavailable => "DATA_ROOT_UNAVAILABLE",
        ErrorCode::AssetCorrupted => "ASSET_CORRUPTED",
        ErrorCode::AssetNotFound => "ASSET_NOT_FOUND",
        _ => "ASSET_IO_FAILED",
    }
}
