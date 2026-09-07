use std::{
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use desktop_notes_core::{AssetFileStore, ErrorCode, SecretKey};
use desktop_notes_infra::{
    EncryptedAssetFiles, ImageInput, ImageInputSource, ImageSourceFormat, MAX_IMAGE_DIMENSION,
    MAX_IMAGE_INPUT_BYTES,
};
use image::{
    ExtendedColorType, ImageEncoder,
    codecs::{jpeg::JpegEncoder, png::PngEncoder, webp::WebPEncoder},
};

const PNG_SIGNATURE: &[u8] = b"\x89PNG\r\n\x1a\n";
const METADATA_MARKER: &[u8] = b"DESKTOP_NOTES_S05_SYNTHETIC_MARKER_7F2B";

#[test]
fn png_jpeg_webp_and_clipboard_screenshot_persist_encrypted_and_restart() {
    let root = isolated_root("formats");
    let store = EncryptedAssetFiles::new(root.clone(), SecretKey::from_bytes([0x71; 32]));
    let rgba = synthetic_rgba(17, 11);
    let plain_png = with_png_text_chunk(encode_png(&rgba, 17, 11), METADATA_MARKER);
    let inputs = [
        (
            "1022d70d-ea85-4acf-8357-835cb2f9af47",
            ImageInputSource::EditorPaste,
            ImageSourceFormat::Png,
            plain_png,
        ),
        (
            "763b47fb-7747-44b2-a780-99093d3a2ca0",
            ImageInputSource::EditorPaste,
            ImageSourceFormat::Jpeg,
            encode_jpeg(&rgba, 17, 11),
        ),
        (
            "fb4aa5ec-81b0-4e81-9aef-2dc3ae39301f",
            ImageInputSource::EditorPaste,
            ImageSourceFormat::WebP,
            encode_webp(&rgba, 17, 11),
        ),
        (
            "436b545e-f187-4a39-8481-e133be3ab8e9",
            ImageInputSource::ClipboardScreenshot,
            ImageSourceFormat::Png,
            encode_png(&rgba, 17, 11),
        ),
    ];

    for (asset_id, source, format, bytes) in inputs {
        let persisted = store
            .persist_image(
                asset_id,
                ImageInput {
                    source,
                    declared_format: format,
                    bytes,
                },
            )
            .unwrap();
        assert_eq!(persisted.record.media_type, "image/png");
        assert_eq!(persisted.record.source_format, format);
        assert_eq!(
            (persisted.record.pixel_width, persisted.record.pixel_height),
            (17, 11)
        );
        assert!(!contains_subslice(
            &persisted.normalized_png,
            METADATA_MARKER
        ));

        let encrypted = fs::read(root.join(&persisted.record.storage_relpath)).unwrap();
        assert!(encrypted.starts_with(b"DNIMGv1\0"));
        assert!(!contains_subslice(&encrypted, PNG_SIGNATURE));
        assert!(!contains_subslice(&encrypted, &persisted.normalized_png));

        let reopened = store
            .read_image(
                asset_id,
                &persisted.record.storage_relpath,
                &persisted.record.sha256_plain,
            )
            .unwrap();
        assert_eq!(reopened, persisted.normalized_png);
    }

    assert_eq!(scan_tree_for_marker(&root, METADATA_MARKER), 0);
    assert!(
        !root.join("staging").exists(),
        "visibility-bound publication must not rely on legacy staging rename"
    );
    remove_test_root(&root);
}

#[test]
fn mismatched_type_spoofed_extension_and_unsupported_screenshot_format_are_rejected() {
    let root = isolated_root("mismatch");
    let store = EncryptedAssetFiles::new(root.clone(), SecretKey::from_bytes([0x72; 32]));
    let rgba = synthetic_rgba(4, 4);
    let png = encode_png(&rgba, 4, 4);

    for (source, format) in [
        (ImageInputSource::EditorPaste, ImageSourceFormat::Jpeg),
        (
            ImageInputSource::ClipboardScreenshot,
            ImageSourceFormat::WebP,
        ),
    ] {
        let error = store
            .persist_image(
                "34d801f1-9be6-4632-a0d5-93369852306a",
                ImageInput {
                    source,
                    declared_format: format,
                    bytes: png.clone(),
                },
            )
            .unwrap_err();
        assert_eq!(error.code(), ErrorCode::ImageRejected);
    }
    assert!(!root.join("assets").exists());
    remove_test_root(&root);
}

#[test]
fn oversized_file_and_unreasonable_dimensions_are_rejected_before_persistence() {
    let root = isolated_root("oversize");
    let store = EncryptedAssetFiles::new(root.clone(), SecretKey::from_bytes([0x73; 32]));
    let too_many_bytes = vec![0_u8; MAX_IMAGE_INPUT_BYTES + 1];
    let error = store
        .persist_image(
            "90241933-4029-4695-9208-658580a444a6",
            ImageInput {
                source: ImageInputSource::EditorPaste,
                declared_format: ImageSourceFormat::Png,
                bytes: too_many_bytes,
            },
        )
        .unwrap_err();
    assert_eq!(error.code(), ErrorCode::ImageTooLarge);

    let dimension_bomb = png_with_dimensions(MAX_IMAGE_DIMENSION, MAX_IMAGE_DIMENSION);
    let error = store
        .persist_image(
            "9bb6f5df-5ced-4bb0-9ac9-62209962811c",
            ImageInput {
                source: ImageInputSource::EditorPaste,
                declared_format: ImageSourceFormat::Png,
                bytes: dimension_bomb,
            },
        )
        .unwrap_err();
    assert_eq!(error.code(), ErrorCode::ImageTooLarge);
    assert!(!root.join("assets").exists());
    remove_test_root(&root);
}

#[test]
fn controlled_asset_identity_prevents_path_traversal_and_collision_overwrite() {
    let root = isolated_root("identity");
    let store = EncryptedAssetFiles::new(root.clone(), SecretKey::from_bytes([0x74; 32]));
    let rgba = synthetic_rgba(3, 2);
    let png = encode_png(&rgba, 3, 2);
    let error = store
        .persist_image(
            "..\\..\\outside.png",
            ImageInput {
                source: ImageInputSource::EditorPaste,
                declared_format: ImageSourceFormat::Png,
                bytes: png.clone(),
            },
        )
        .unwrap_err();
    assert_eq!(error.code(), ErrorCode::ValidationFailed);

    let asset_id = "f5facd3d-eaa9-4f07-845d-72a749081a8d";
    let first = store
        .persist_image(
            asset_id,
            ImageInput {
                source: ImageInputSource::EditorPaste,
                declared_format: ImageSourceFormat::Png,
                bytes: png.clone(),
            },
        )
        .unwrap();
    let original_cipher = fs::read(root.join(&first.record.storage_relpath)).unwrap();
    let error = store
        .persist_image(
            asset_id,
            ImageInput {
                source: ImageInputSource::EditorPaste,
                declared_format: ImageSourceFormat::Png,
                bytes: png,
            },
        )
        .unwrap_err();
    assert_eq!(error.code(), ErrorCode::DataRootUnavailable);
    assert_eq!(
        fs::read(root.join(&first.record.storage_relpath)).unwrap(),
        original_cipher
    );
    remove_test_root(&root);
}

#[test]
fn maintenance_only_removes_recognized_staging_and_quarantines_unknown_asset_containers() {
    let root = isolated_root("maintenance-scope");
    let store = EncryptedAssetFiles::new(root.clone(), SecretKey::from_bytes([0x75; 32]));
    let staging = root.join("staging");
    let assets = root.join("assets").join("misc");
    fs::create_dir_all(&staging).unwrap();
    fs::create_dir_all(&assets).unwrap();
    let recognized = staging.join("f5facd3d-eaa9-4f07-845d-72a749081a8d-0123456789abcdef.part");
    let unrelated_part = staging.join("user-owned.part");
    let unknown_container = assets.join("unknown.dnimg");
    let unrelated_asset_file = assets.join("keep.txt");
    fs::write(&recognized, b"partial encrypted asset").unwrap();
    fs::write(&unrelated_part, b"unrelated").unwrap();
    fs::write(&unknown_container, b"unknown encrypted container").unwrap();
    fs::write(&unrelated_asset_file, b"unrelated").unwrap();

    assert_eq!(store.cleanup_staging().unwrap(), 1);
    assert!(!recognized.exists());
    assert!(unrelated_part.exists());
    assert_eq!(
        store
            .quarantine_unknown_assets(&std::collections::HashSet::new())
            .unwrap(),
        1
    );
    assert!(!unknown_container.exists());
    assert!(unrelated_asset_file.exists());
    assert_eq!(
        fs::read_dir(staging.join("quarantine"))
            .unwrap()
            .filter_map(Result::ok)
            .count(),
        1
    );
    remove_test_root(&root);
}

#[test]
fn product_read_rejects_database_dimension_mismatch() {
    let root = isolated_root("dimension-integrity");
    let store = EncryptedAssetFiles::new(root.clone(), SecretKey::from_bytes([0x76; 32]));
    let asset_id = "a5facd3d-eaa9-4f07-845d-72a749081a8d";
    let rgba = synthetic_rgba(3, 2);
    let persisted = store
        .persist_image(
            asset_id,
            ImageInput {
                source: ImageInputSource::EditorPaste,
                declared_format: ImageSourceFormat::Png,
                bytes: encode_png(&rgba, 3, 2),
            },
        )
        .unwrap();

    let error = AssetFileStore::read_image(
        &store,
        asset_id,
        &persisted.record.storage_relpath,
        &persisted.record.sha256_plain,
        4,
        2,
    )
    .unwrap_err();
    assert_eq!(error.code(), ErrorCode::AssetCorrupted);
    remove_test_root(&root);
}

fn synthetic_rgba(width: u32, height: u32) -> Vec<u8> {
    let mut bytes = Vec::with_capacity((width * height * 4) as usize);
    for y in 0..height {
        for x in 0..width {
            bytes.extend_from_slice(&[
                (x * 13 + y * 7) as u8,
                (x * 3 + y * 17) as u8,
                (x * 19 + y * 5) as u8,
                255,
            ]);
        }
    }
    bytes
}

fn encode_png(rgba: &[u8], width: u32, height: u32) -> Vec<u8> {
    let mut output = Vec::new();
    PngEncoder::new(&mut output)
        .write_image(rgba, width, height, ExtendedColorType::Rgba8)
        .unwrap();
    output
}

fn encode_jpeg(rgba: &[u8], width: u32, height: u32) -> Vec<u8> {
    let rgb = rgba
        .as_chunks::<4>()
        .0
        .iter()
        .flat_map(|pixel| [pixel[0], pixel[1], pixel[2]])
        .collect::<Vec<_>>();
    let mut output = Vec::new();
    JpegEncoder::new_with_quality(&mut output, 92)
        .write_image(&rgb, width, height, ExtendedColorType::Rgb8)
        .unwrap();
    output
}

fn encode_webp(rgba: &[u8], width: u32, height: u32) -> Vec<u8> {
    let mut output = Vec::new();
    WebPEncoder::new_lossless(&mut output)
        .write_image(rgba, width, height, ExtendedColorType::Rgba8)
        .unwrap();
    output
}

fn with_png_text_chunk(mut png: Vec<u8>, marker: &[u8]) -> Vec<u8> {
    let iend = png.len() - 12;
    let mut data = b"marker\0".to_vec();
    data.extend_from_slice(marker);
    let mut chunk = Vec::new();
    chunk.extend_from_slice(&(data.len() as u32).to_be_bytes());
    chunk.extend_from_slice(b"tEXt");
    chunk.extend_from_slice(&data);
    let mut crc_input = b"tEXt".to_vec();
    crc_input.extend_from_slice(&data);
    chunk.extend_from_slice(&crc32(&crc_input).to_be_bytes());
    png.splice(iend..iend, chunk);
    png
}

fn png_with_dimensions(width: u32, height: u32) -> Vec<u8> {
    let mut png = PNG_SIGNATURE.to_vec();
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&width.to_be_bytes());
    ihdr.extend_from_slice(&height.to_be_bytes());
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]);
    append_png_chunk(&mut png, b"IHDR", &ihdr);
    // A complete empty zlib stream is sufficient for decoder construction;
    // the production dimension gate must reject before pixel allocation/read.
    append_png_chunk(
        &mut png,
        b"IDAT",
        &[
            0x78, 0x01, 0x01, 0x00, 0x00, 0xff, 0xff, 0x00, 0x00, 0x00, 0x01,
        ],
    );
    append_png_chunk(&mut png, b"IEND", &[]);
    png
}

fn append_png_chunk(png: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    png.extend_from_slice(&(data.len() as u32).to_be_bytes());
    png.extend_from_slice(kind);
    png.extend_from_slice(data);
    let mut crc_input = kind.to_vec();
    crc_input.extend_from_slice(data);
    png.extend_from_slice(&crc32(&crc_input).to_be_bytes());
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = 0xffff_ffff_u32;
    for byte in bytes {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xedb8_8320_u32 & 0_u32.wrapping_sub(crc & 1));
        }
    }
    !crc
}

fn isolated_root(label: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "desktop-notes-s05-{label}-{}-{nonce}",
        std::process::id()
    ));
    fs::create_dir_all(&root).unwrap();
    root
}

fn remove_test_root(root: &Path) {
    assert!(root.starts_with(std::env::temp_dir()));
    assert!(
        root.file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with("desktop-notes-s05-"))
    );
    fs::remove_dir_all(root).unwrap();
}

fn scan_tree_for_marker(root: &Path, marker: &[u8]) -> usize {
    let mut hits = 0;
    let mut pending = vec![root.to_path_buf()];
    while let Some(path) = pending.pop() {
        for entry in fs::read_dir(path).unwrap() {
            let entry = entry.unwrap();
            if entry.file_type().unwrap().is_dir() {
                pending.push(entry.path());
            } else if contains_subslice(&fs::read(entry.path()).unwrap(), marker) {
                hits += 1;
            }
        }
    }
    hits
}

fn contains_subslice(haystack: &[u8], needle: &[u8]) -> bool {
    !needle.is_empty()
        && haystack
            .windows(needle.len())
            .any(|window| window == needle)
}
