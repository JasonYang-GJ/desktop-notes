use std::{collections::HashSet, sync::Mutex};

use base64::{Engine as _, engine::general_purpose::STANDARD};
use desktop_notes_core::{
    AssetFileStore, AssetIdGenerator, AssetRecord, AssetStore, Clock, ErrorCode, FoundationError,
    ImageAssetService, ImageInput, ImageSourceFormat, PersistedImage, StagedAssetImport,
};
use desktop_notes_desktop::ipc::{handle_import_image_asset, handle_read_image_asset};
use serde_json::json;

const NOTE_ID: &str = "71000000-0000-4000-8000-000000000001";
const ASSET_ID: &str = "71000000-0000-4000-8000-000000000002";
const IMPORT_ID: &str = "71000000-0000-4000-8000-000000000003";

#[derive(Default)]
struct MemoryAssets {
    staged: Mutex<Option<StagedAssetImport>>,
    ready: Mutex<bool>,
}

impl AssetStore for MemoryAssets {
    fn find_staged_import(
        &self,
        client_import_id: &str,
    ) -> Result<Option<StagedAssetImport>, FoundationError> {
        Ok(self
            .staged
            .lock()
            .unwrap()
            .as_ref()
            .filter(|value| value.client_import_id == client_import_id)
            .cloned())
    }

    fn stage_import(&self, staged: StagedAssetImport) -> Result<(), FoundationError> {
        *self.staged.lock().unwrap() = Some(staged);
        Ok(())
    }

    fn discard_staged_import(
        &self,
        client_import_id: &str,
    ) -> Result<Option<StagedAssetImport>, FoundationError> {
        let mut staged = self.staged.lock().unwrap();
        if staged.as_ref().map(|value| value.client_import_id.as_str()) == Some(client_import_id) {
            Ok(staged.take())
        } else {
            Ok(None)
        }
    }

    fn get_ready_asset_for_note(
        &self,
        note_id: &str,
        asset_id: &str,
    ) -> Result<Option<AssetRecord>, FoundationError> {
        Ok(
            (*self.ready.lock().unwrap() && note_id == NOTE_ID && asset_id == ASSET_ID)
                .then(record),
        )
    }

    fn tracked_storage_relpaths(&self) -> Result<Vec<String>, FoundationError> {
        Ok(Vec::new())
    }

    fn list_due_gc_assets(
        &self,
        _now_ms: i64,
        _limit: u32,
    ) -> Result<Vec<AssetRecord>, FoundationError> {
        Ok(Vec::new())
    }

    fn finalize_gc_asset(&self, _asset_id: &str) -> Result<bool, FoundationError> {
        Ok(false)
    }

    fn record_gc_failure(
        &self,
        _asset_id: &str,
        _error_code: ErrorCode,
    ) -> Result<(), FoundationError> {
        Ok(())
    }
}

struct MemoryFiles;

impl AssetFileStore for MemoryFiles {
    fn persist_image(
        &self,
        _asset_id: &str,
        input: ImageInput,
    ) -> Result<PersistedImage, FoundationError> {
        Ok(PersistedImage {
            record: record(),
            normalized_png: input.bytes,
        })
    }

    fn read_image(
        &self,
        _asset_id: &str,
        _storage_relpath: &str,
        _expected_sha256: &[u8; 32],
        _expected_pixel_width: u32,
        _expected_pixel_height: u32,
    ) -> Result<Vec<u8>, FoundationError> {
        Ok(vec![1, 2, 3, 4])
    }

    fn remove_image(&self, _asset_id: &str, _storage_relpath: &str) -> Result<(), FoundationError> {
        Ok(())
    }

    fn cleanup_staging(&self) -> Result<u32, FoundationError> {
        Ok(0)
    }

    fn quarantine_unknown_assets(
        &self,
        _tracked_storage_relpaths: &HashSet<String>,
    ) -> Result<u32, FoundationError> {
        Ok(0)
    }
}

struct FixedRuntime;

impl AssetIdGenerator for FixedRuntime {
    fn new_asset_id(&self) -> Result<String, FoundationError> {
        Ok(ASSET_ID.to_owned())
    }
}

impl Clock for FixedRuntime {
    fn now_utc_ms(&self) -> Result<i64, FoundationError> {
        Ok(4_000)
    }
}

#[test]
fn typed_import_accepts_only_approved_bounded_image_payloads() {
    let assets = MemoryAssets::default();
    let service = ImageAssetService::new(&assets, &MemoryFiles, &FixedRuntime, &FixedRuntime);
    let response = serde_json::to_value(handle_import_image_asset(
        json!({
            "protocolVersion": 1,
            "clientImportId": IMPORT_ID,
            "mediaType": "image/png",
            "source": "clipboard_screenshot",
            "dataBase64": STANDARD.encode([1, 2, 3, 4]),
        }),
        &service,
    ))
    .unwrap();
    assert_eq!(response["ok"], true);
    assert_eq!(response["image"]["assetId"], ASSET_ID);
    assert_eq!(response["image"]["mediaType"], "image/png");
    assert_eq!(
        response["image"]["dataBase64"],
        STANDARD.encode([1, 2, 3, 4])
    );
    assert!(response.to_string().find("storage_relpath").is_none());
    assert!(response.to_string().find("key_id").is_none());

    for invalid in [
        json!({
            "protocolVersion": 1,
            "clientImportId": IMPORT_ID,
            "mediaType": "image/svg+xml",
            "source": "editor_paste",
            "dataBase64": STANDARD.encode(b"<svg onload=alert(1)>")
        }),
        json!({
            "protocolVersion": 1,
            "clientImportId": IMPORT_ID,
            "mediaType": "image/png",
            "source": "editor_paste",
            "dataBase64": "%%%"
        }),
        json!({
            "protocolVersion": 1,
            "clientImportId": IMPORT_ID,
            "mediaType": "image/png",
            "source": "editor_paste",
            "dataBase64": STANDARD.encode([1]),
            "absolutePath": "/synthetic/untrusted.png"
        }),
    ] {
        let rejected = serde_json::to_value(handle_import_image_asset(invalid, &service)).unwrap();
        assert_eq!(rejected["ok"], false);
        assert!(matches!(
            rejected["error"]["code"].as_str(),
            Some("IMAGE_REJECTED" | "VALIDATION_FAILED")
        ));
    }
}

#[test]
fn read_is_scoped_to_note_and_asset_and_returns_no_filesystem_capability() {
    let assets = MemoryAssets::default();
    *assets.ready.lock().unwrap() = true;
    let service = ImageAssetService::new(&assets, &MemoryFiles, &FixedRuntime, &FixedRuntime);
    let response = serde_json::to_value(handle_read_image_asset(
        json!({ "protocolVersion": 1, "noteId": NOTE_ID, "assetId": ASSET_ID }),
        &service,
    ))
    .unwrap();
    assert_eq!(response["ok"], true);
    assert_eq!(
        response["image"]["dataBase64"],
        STANDARD.encode([1, 2, 3, 4])
    );

    let denied = serde_json::to_value(handle_read_image_asset(
        json!({
            "protocolVersion": 1,
            "noteId": "71000000-0000-4000-8000-000000000099",
            "assetId": ASSET_ID
        }),
        &service,
    ))
    .unwrap();
    assert_eq!(denied["ok"], false);
    assert_eq!(denied["error"]["code"], "ASSET_NOT_FOUND");
}

fn record() -> AssetRecord {
    AssetRecord {
        asset_id: ASSET_ID.to_owned(),
        media_type: "image/png".to_owned(),
        source_format: ImageSourceFormat::Png,
        storage_relpath: format!("assets/71/00/{ASSET_ID}.dnimg"),
        byte_size_plain: 4,
        byte_size_cipher: 80,
        pixel_width: 2,
        pixel_height: 2,
        sha256_plain: [0x55; 32],
        crypto_format_version: 1,
        key_id: "assets-v1".to_owned(),
        created_at_ms: 4_000,
    }
}
