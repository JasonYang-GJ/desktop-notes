use std::{
    collections::HashSet,
    env, fs,
    io::Write,
    path::{Path, PathBuf},
};

use desktop_notes_core::SecretKey;
use desktop_notes_infra::{EncryptedAssetFiles, ImageInput, ImageInputSource, ImageSourceFormat};
use image::{
    ExtendedColorType, ImageEncoder,
    codecs::{jpeg::JpegEncoder, png::PngEncoder, webp::WebPEncoder},
};
use serde_json::json;

const MARKER: &[u8] = b"DESKTOP_NOTES_S05_SYNTHETIC_MARKER_7F2B";

fn main() {
    if let Err(message) = run() {
        eprintln!("{message}");
        std::process::exit(2);
    }
}

fn run() -> Result<(), &'static str> {
    let args = env::args().skip(1).collect::<Vec<_>>();
    match args.first().map(String::as_str) {
        Some("fixtures") if args.len() == 2 => create_fixtures(Path::new(&args[1])),
        Some("write") if args.len() == 7 => write_asset(&args[1..]),
        Some("read") if args.len() == 5 => read_asset(&args[1..]),
        Some("read-file") if args.len() == 6 => read_asset_to_file(&args[1..]),
        Some("quarantine") if args.len() == 3 => quarantine_assets(&args[1..]),
        Some("scan") if args.len() >= 6 => scan_scopes(&args[1..]),
        _ => Err("invalid S05 harness arguments"),
    }
}

fn quarantine_assets(args: &[String]) -> Result<(), &'static str> {
    let tracked = fs::read_to_string(&args[1])
        .map_err(|_| "tracked asset list unavailable")?
        .lines()
        .filter(|line| !line.is_empty())
        .map(str::to_owned)
        .collect::<HashSet<_>>();
    let quarantined = synthetic_store(&args[0])
        .quarantine_unknown_assets(&tracked)
        .map_err(|_| "asset quarantine failed")?;
    println!("{}", json!({"pass": true, "quarantined": quarantined}));
    Ok(())
}

fn synthetic_store(root: &str) -> EncryptedAssetFiles {
    EncryptedAssetFiles::new(PathBuf::from(root), SecretKey::from_bytes([0xa5; 32]))
}

fn create_fixtures(root: &Path) -> Result<(), &'static str> {
    fs::create_dir_all(root).map_err(|_| "fixture directory unavailable")?;
    let rgba = synthetic_rgba(37, 23);
    let png = with_png_text_chunk(encode_png(&rgba, 37, 23), MARKER);
    fs::write(root.join("synthetic-marker.png"), png).map_err(|_| "PNG fixture write failed")?;
    fs::write(
        root.join("synthetic-marker.jpeg"),
        encode_jpeg(&rgba, 37, 23),
    )
    .map_err(|_| "JPEG fixture write failed")?;
    fs::write(
        root.join("synthetic-marker.webp"),
        encode_webp(&rgba, 37, 23),
    )
    .map_err(|_| "WebP fixture write failed")?;
    println!("{}", json!({"pass": true, "width": 37, "height": 23}));
    Ok(())
}

fn write_asset(args: &[String]) -> Result<(), &'static str> {
    let root = &args[0];
    let asset_id = &args[1];
    let input_path = &args[2];
    let format = parse_format(&args[3])?;
    let source = match args[4].as_str() {
        "editor" => ImageInputSource::EditorPaste,
        "screenshot" => ImageInputSource::ClipboardScreenshot,
        _ => return Err("invalid input source"),
    };
    let metadata_path = &args[5];
    let bytes = fs::read(input_path).map_err(|_| "fixture read failed")?;
    let persisted = synthetic_store(root)
        .persist_image(
            asset_id,
            ImageInput {
                source,
                declared_format: format,
                bytes,
            },
        )
        .map_err(|_| "asset persistence failed")?;
    let result = json!({
        "pass": true,
        "asset_id": persisted.record.asset_id,
        "media_type": persisted.record.media_type,
        "source_format": persisted.record.source_format.as_str(),
        "storage_relpath": persisted.record.storage_relpath,
        "byte_size_plain": persisted.record.byte_size_plain,
        "byte_size_cipher": persisted.record.byte_size_cipher,
        "pixel_width": persisted.record.pixel_width,
        "pixel_height": persisted.record.pixel_height,
        "sha256_plain": hex(&persisted.record.sha256_plain),
        "crypto_format_version": persisted.record.crypto_format_version,
        "key_id": persisted.record.key_id,
        "metadata_marker_stripped": !contains(&persisted.normalized_png, MARKER),
    });
    fs::write(
        metadata_path,
        serde_json::to_vec_pretty(&result).map_err(|_| "metadata encode failed")?,
    )
    .map_err(|_| "metadata write failed")?;
    println!("{result}");
    Ok(())
}

fn read_asset(args: &[String]) -> Result<(), &'static str> {
    let expected_sha = decode_hex_32(&args[3])?;
    let plaintext = synthetic_store(&args[0])
        .read_image(&args[1], &args[2], &expected_sha)
        .map_err(|_| "asset read failed")?;
    std::io::stdout()
        .write_all(&plaintext)
        .map_err(|_| "stdout write failed")
}

fn read_asset_to_file(args: &[String]) -> Result<(), &'static str> {
    let expected_sha = decode_hex_32(&args[3])?;
    let plaintext = synthetic_store(&args[0])
        .read_image(&args[1], &args[2], &expected_sha)
        .map_err(|_| "asset read failed")?;
    fs::write(&args[4], plaintext.as_slice()).map_err(|_| "plaintext fixture write failed")
}

fn scan_scopes(args: &[String]) -> Result<(), &'static str> {
    let expected_sha = decode_hex_32(&args[3])?;
    let plaintext = synthetic_store(&args[0])
        .read_image(&args[1], &args[2], &expected_sha)
        .map_err(|_| "asset read failed")?;
    let mut plaintext_hits = Vec::new();
    let mut marker_hits = Vec::new();
    let mut scanned_files = 0_u64;
    for scope in &args[4..] {
        scan_path(
            Path::new(scope),
            &plaintext,
            &mut plaintext_hits,
            &mut marker_hits,
            &mut scanned_files,
        )?;
    }
    println!(
        "{}",
        json!({
            "pass": plaintext_hits.is_empty() && marker_hits.is_empty(),
            "scanned_files": scanned_files,
            "plaintext_hits": plaintext_hits,
            "marker_hits": marker_hits,
        })
    );
    Ok(())
}

fn scan_path(
    path: &Path,
    plaintext: &[u8],
    plaintext_hits: &mut Vec<String>,
    marker_hits: &mut Vec<String>,
    scanned_files: &mut u64,
) -> Result<(), &'static str> {
    if !path.exists() {
        return Ok(());
    }
    if path.is_dir() {
        for entry in fs::read_dir(path).map_err(|_| "scan directory read failed")? {
            let entry = entry.map_err(|_| "scan entry read failed")?;
            scan_path(
                &entry.path(),
                plaintext,
                plaintext_hits,
                marker_hits,
                scanned_files,
            )?;
        }
        return Ok(());
    }
    *scanned_files += 1;
    let bytes = fs::read(path).map_err(|_| "scan file read failed")?;
    let display = path.to_string_lossy().into_owned();
    if contains(&bytes, plaintext) {
        plaintext_hits.push(display.clone());
    }
    if contains(&bytes, MARKER) {
        marker_hits.push(display);
    }
    Ok(())
}

fn parse_format(value: &str) -> Result<ImageSourceFormat, &'static str> {
    match value {
        "png" => Ok(ImageSourceFormat::Png),
        "jpeg" => Ok(ImageSourceFormat::Jpeg),
        "webp" => Ok(ImageSourceFormat::WebP),
        _ => Err("invalid image format"),
    }
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
        .expect("bounded synthetic PNG encodes");
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
        .expect("bounded synthetic JPEG encodes");
    output
}

fn encode_webp(rgba: &[u8], width: u32, height: u32) -> Vec<u8> {
    let mut output = Vec::new();
    WebPEncoder::new_lossless(&mut output)
        .write_image(rgba, width, height, ExtendedColorType::Rgba8)
        .expect("bounded synthetic WebP encodes");
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

fn decode_hex_32(value: &str) -> Result<[u8; 32], &'static str> {
    if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("invalid SHA-256");
    }
    let mut output = [0_u8; 32];
    for (index, chunk) in value.as_bytes().as_chunks::<2>().0.iter().enumerate() {
        output[index] = (hex_value(chunk[0])? << 4) | hex_value(chunk[1])?;
    }
    Ok(output)
}

fn hex_value(value: u8) -> Result<u8, &'static str> {
    match value {
        b'0'..=b'9' => Ok(value - b'0'),
        b'a'..=b'f' => Ok(value - b'a' + 10),
        b'A'..=b'F' => Ok(value - b'A' + 10),
        _ => Err("invalid hex"),
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    !needle.is_empty()
        && haystack
            .windows(needle.len())
            .any(|window| window == needle)
}
