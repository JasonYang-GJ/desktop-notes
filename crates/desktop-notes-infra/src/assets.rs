use std::{
    fs::{self, File, OpenOptions},
    io::{self, Cursor, Read, Write},
    path::{Path, PathBuf},
    sync::{RwLock, RwLockReadGuard},
};

use aes_gcm::{
    Aes256Gcm, Nonce,
    aead::{Aead, KeyInit, Payload},
};
use desktop_notes_core::{
    AssetFileStore, AssetRecord, ErrorCode, FoundationError, SecretKey, validate_note_id,
};
pub use desktop_notes_core::{ImageInput, ImageInputSource, ImageSourceFormat, PersistedImage};
use hkdf::Hkdf;
use image::{
    DynamicImage, GenericImageView, ImageDecoder, ImageEncoder, ImageFormat, ImageReader, Limits,
    codecs::png::PngEncoder,
};
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

const DNIMG_MAGIC: &[u8; 8] = b"DNIMGv1\0";
const DNIMG_ALGORITHM_AES_256_GCM: u8 = 1;
const DNIMG_KEY_ID: &[u8; 8] = b"assetsv1";
const DNIMG_SALT_LEN: usize = 16;
const DNIMG_NONCE_PREFIX_LEN: usize = 8;
const DNIMG_TAG_LEN: usize = 16;
const DNIMG_HEADER_LEN: usize = 8 + 2 + 1 + 8 + 4 + DNIMG_SALT_LEN + DNIMG_NONCE_PREFIX_LEN + 8;
const DNIMG_CONTAINER_KEY_INFO: &[u8] = b"dnimg-container-v1";
const PNG_MEDIA_TYPE: &str = "image/png";

pub const DNIMG_FORMAT_VERSION: u16 = 1;
pub const DNIMG_CHUNK_SIZE: u32 = 64 * 1024;
pub const MAX_IMAGE_INPUT_BYTES: usize = 20 * 1024 * 1024;
pub const MAX_NORMALIZED_IMAGE_BYTES: usize = 32 * 1024 * 1024;
pub const MAX_IMAGE_DIMENSION: u32 = 8192;
pub const MAX_IMAGE_PIXELS: u64 = 16_777_216;

pub struct EncryptedAssetFiles {
    data_root: PathBuf,
    asset_key: SecretKey,
    access: RwLock<()>,
}

impl EncryptedAssetFiles {
    pub fn new(data_root: PathBuf, asset_key: SecretKey) -> Self {
        Self {
            data_root,
            asset_key,
            access: RwLock::new(()),
        }
    }

    pub fn data_root(&self) -> &Path {
        &self.data_root
    }

    pub fn persist_image(
        &self,
        asset_id: &str,
        input: ImageInput,
    ) -> Result<PersistedImage, FoundationError> {
        let _access = self.access.write().map_err(|_| data_root_unavailable())?;
        validate_note_id(asset_id)?;
        let normalized = normalize_image(input)?;
        let storage_relpath = storage_relative_path(asset_id)?;
        let final_path = self.data_root.join(Path::new(&storage_relpath));
        let final_parent = final_path.parent().ok_or_else(data_root_unavailable)?;
        fs::create_dir_all(final_parent).map_err(map_asset_io)?;

        let mut final_created = false;
        let result = (|| {
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&final_path)
                .map_err(map_asset_io)?;
            final_created = true;
            write_encrypted_container(
                &self.asset_key,
                asset_id,
                normalized.bytes.as_slice(),
                &mut file,
            )?;
            file.sync_all().map_err(map_asset_io)?;
            drop(file);

            let verified = decrypt_container_file(&self.asset_key, asset_id, &final_path)?;
            if verified.as_slice() != normalized.bytes.as_slice() {
                return Err(FoundationError::asset_corrupted());
            }
            drop(verified);

            let byte_size_cipher = fs::metadata(&final_path).map_err(map_asset_io)?.len();
            Ok(PersistedImage {
                record: AssetRecord {
                    asset_id: asset_id.to_owned(),
                    media_type: PNG_MEDIA_TYPE.to_owned(),
                    source_format: normalized.source_format,
                    storage_relpath,
                    byte_size_plain: normalized.bytes.len() as u64,
                    byte_size_cipher,
                    pixel_width: normalized.pixel_width,
                    pixel_height: normalized.pixel_height,
                    sha256_plain: normalized.sha256,
                    crypto_format_version: DNIMG_FORMAT_VERSION,
                    key_id: "assets-v1".to_owned(),
                    created_at_ms: 0,
                },
                normalized_png: normalized.bytes,
            })
        })();
        if result.is_err() && final_created {
            let _ = fs::remove_file(&final_path);
        }
        result
    }

    pub fn read_image(
        &self,
        asset_id: &str,
        storage_relpath: &str,
        expected_sha256: &[u8; 32],
    ) -> Result<Vec<u8>, FoundationError> {
        let _access = self.access.read().map_err(|_| data_root_unavailable())?;
        let expected_relpath = storage_relative_path(asset_id)?;
        if storage_relpath != expected_relpath {
            return Err(FoundationError::asset_corrupted());
        }
        let path = self.data_root.join(Path::new(storage_relpath));
        let plaintext = decrypt_container_file(&self.asset_key, asset_id, &path)?;
        let actual_sha256: [u8; 32] = Sha256::digest(&plaintext).into();
        if &actual_sha256 != expected_sha256 {
            return Err(FoundationError::asset_corrupted());
        }
        validate_normalized_png(&plaintext)?;
        Ok(plaintext.to_vec())
    }

    pub fn remove_image(
        &self,
        asset_id: &str,
        storage_relpath: &str,
    ) -> Result<(), FoundationError> {
        let _access = self.access.write().map_err(|_| data_root_unavailable())?;
        let expected_relpath = storage_relative_path(asset_id)?;
        if storage_relpath != expected_relpath {
            return Err(FoundationError::asset_corrupted());
        }
        let path = self.data_root.join(Path::new(storage_relpath));
        match fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(map_asset_io(error)),
        }
    }

    pub fn cleanup_staging(&self) -> Result<u32, FoundationError> {
        let _access = self.access.write().map_err(|_| data_root_unavailable())?;
        let staging = self.data_root.join("staging");
        let entries = match fs::read_dir(&staging) {
            Ok(entries) => entries,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(0),
            Err(error) => return Err(map_asset_io(error)),
        };
        let mut removed = 0_u32;
        for entry in entries {
            let entry = entry.map_err(map_asset_io)?;
            if entry.file_type().map_err(map_asset_io)?.is_file()
                && entry
                    .file_name()
                    .to_str()
                    .is_some_and(is_asset_staging_name)
            {
                fs::remove_file(entry.path()).map_err(map_asset_io)?;
                removed = removed.saturating_add(1);
            }
        }
        Ok(removed)
    }

    pub fn quarantine_unknown_assets(
        &self,
        tracked_storage_relpaths: &std::collections::HashSet<String>,
    ) -> Result<u32, FoundationError> {
        let _access = self.access.write().map_err(|_| data_root_unavailable())?;
        let assets_root = self.data_root.join("assets");
        if !assets_root.exists() {
            return Ok(0);
        }
        let quarantine_root = self.data_root.join("staging").join("quarantine");
        let mut pending = vec![assets_root];
        let mut quarantined = 0_u32;
        while let Some(directory) = pending.pop() {
            for entry in fs::read_dir(directory).map_err(map_asset_io)? {
                let entry = entry.map_err(map_asset_io)?;
                if entry.file_type().map_err(map_asset_io)?.is_dir() {
                    pending.push(entry.path());
                    continue;
                }
                if entry.path().extension().and_then(|value| value.to_str()) != Some("dnimg") {
                    continue;
                }
                let relative = entry
                    .path()
                    .strip_prefix(&self.data_root)
                    .map_err(|_| FoundationError::asset_corrupted())?
                    .to_string_lossy()
                    .replace('\\', "/");
                if tracked_storage_relpaths.contains(&relative) {
                    continue;
                }
                fs::create_dir_all(&quarantine_root).map_err(map_asset_io)?;
                let mut suffix = [0_u8; 8];
                getrandom::fill(&mut suffix).map_err(|_| asset_crypto_unavailable())?;
                let destination = quarantine_root.join(format!(
                    "{}-{}.orphan",
                    entry.file_name().to_string_lossy(),
                    hex(&suffix)
                ));
                quarantine_file(&entry.path(), &destination)?;
                quarantined = quarantined.saturating_add(1);
            }
        }
        Ok(quarantined)
    }

    pub(crate) fn backup_read_guard(&self) -> Result<AssetBackupReadGuard<'_>, FoundationError> {
        Ok(AssetBackupReadGuard {
            store: self,
            _access: self.access.read().map_err(|_| data_root_unavailable())?,
        })
    }
}

pub(crate) struct AssetBackupReadGuard<'a> {
    store: &'a EncryptedAssetFiles,
    _access: RwLockReadGuard<'a, ()>,
}

impl AssetBackupReadGuard<'_> {
    pub(crate) fn source_path(
        &self,
        asset_id: &str,
        storage_relpath: &str,
        expected_size: u64,
    ) -> Result<PathBuf, FoundationError> {
        let file = self.open_ciphertext(asset_id, storage_relpath, expected_size)?;
        drop(file);
        Ok(self.store.data_root.join(Path::new(storage_relpath)))
    }

    pub(crate) fn open_ciphertext(
        &self,
        asset_id: &str,
        storage_relpath: &str,
        expected_size: u64,
    ) -> Result<File, FoundationError> {
        if storage_relpath != storage_relative_path(asset_id)? {
            return Err(FoundationError::backup_corrupted());
        }
        let path = self.store.data_root.join(Path::new(storage_relpath));
        let file = File::open(path).map_err(map_asset_io)?;
        let metadata = file.metadata().map_err(map_asset_io)?;
        if !metadata.is_file() || metadata.len() != expected_size {
            return Err(FoundationError::backup_corrupted());
        }
        Ok(file)
    }
}

fn write_encrypted_container<W: Write>(
    asset_key: &SecretKey,
    asset_id: &str,
    plaintext: &[u8],
    output: &mut W,
) -> Result<(), FoundationError> {
    if plaintext.is_empty() || plaintext.len() > MAX_NORMALIZED_IMAGE_BYTES {
        return Err(FoundationError::image_too_large());
    }
    let mut salt = [0_u8; DNIMG_SALT_LEN];
    let mut nonce_prefix = [0_u8; DNIMG_NONCE_PREFIX_LEN];
    getrandom::fill(&mut salt).map_err(|_| asset_crypto_unavailable())?;
    getrandom::fill(&mut nonce_prefix).map_err(|_| asset_crypto_unavailable())?;
    let header = encode_header(&salt, &nonce_prefix, plaintext.len() as u64);
    let header_hash: [u8; 32] = Sha256::digest(&header).into();
    let container_key = derive_container_key(asset_key, asset_id, &salt)?;
    let cipher = Aes256Gcm::new_from_slice(container_key.as_slice())
        .map_err(|_| asset_crypto_unavailable())?;

    output.write_all(&header).map_err(map_asset_io)?;
    for (index, chunk) in plaintext.chunks(DNIMG_CHUNK_SIZE as usize).enumerate() {
        let chunk_index = u32::try_from(index).map_err(|_| FoundationError::image_too_large())?;
        let nonce = chunk_nonce(&nonce_prefix, chunk_index);
        let aad = chunk_aad(
            &header_hash,
            asset_id,
            chunk_index,
            chunk.len() as u32,
            plaintext.len() as u64,
        );
        let encrypted = cipher
            .encrypt(
                &nonce,
                Payload {
                    msg: chunk,
                    aad: &aad,
                },
            )
            .map_err(|_| asset_crypto_unavailable())?;
        output.write_all(&encrypted).map_err(map_asset_io)?;
    }
    Ok(())
}

fn quarantine_file(source: &Path, destination: &Path) -> Result<(), FoundationError> {
    match fs::rename(source, destination) {
        Ok(()) => Ok(()),
        Err(error)
            if error.kind() == io::ErrorKind::CrossesDevices
                || error.raw_os_error() == Some(17) =>
        {
            quarantine_file_by_copy(source, destination)
        }
        Err(error) => Err(map_asset_io(error)),
    }
}

fn quarantine_file_by_copy(source: &Path, destination: &Path) -> Result<(), FoundationError> {
    let mut destination_created = false;
    let result = (|| {
        let input = File::open(source).map_err(map_asset_io)?;
        let source_length = input.metadata().map_err(map_asset_io)?.len();
        let maximum_container_length =
            (DNIMG_HEADER_LEN + MAX_NORMALIZED_IMAGE_BYTES + 8192) as u64;
        if source_length > maximum_container_length {
            return Err(FoundationError::asset_corrupted());
        }
        let mut output = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(destination)
            .map_err(map_asset_io)?;
        destination_created = true;
        let copied = io::copy(&mut input.take(maximum_container_length + 1), &mut output)
            .map_err(map_asset_io)?;
        if copied != source_length {
            return Err(FoundationError::asset_corrupted());
        }
        output.sync_all().map_err(map_asset_io)?;
        drop(output);
        fs::remove_file(source).map_err(map_asset_io)
    })();
    if result.is_err() && destination_created {
        let _ = fs::remove_file(destination);
    }
    result
}

impl AssetFileStore for EncryptedAssetFiles {
    fn persist_image(
        &self,
        asset_id: &str,
        input: ImageInput,
    ) -> Result<PersistedImage, FoundationError> {
        EncryptedAssetFiles::persist_image(self, asset_id, input)
    }

    fn read_image(
        &self,
        asset_id: &str,
        storage_relpath: &str,
        expected_sha256: &[u8; 32],
        expected_pixel_width: u32,
        expected_pixel_height: u32,
    ) -> Result<Vec<u8>, FoundationError> {
        let bytes =
            EncryptedAssetFiles::read_image(self, asset_id, storage_relpath, expected_sha256)?;
        if normalized_png_dimensions(&bytes)? != (expected_pixel_width, expected_pixel_height) {
            return Err(FoundationError::asset_corrupted());
        }
        Ok(bytes)
    }

    fn remove_image(&self, asset_id: &str, storage_relpath: &str) -> Result<(), FoundationError> {
        EncryptedAssetFiles::remove_image(self, asset_id, storage_relpath)
    }

    fn cleanup_staging(&self) -> Result<u32, FoundationError> {
        EncryptedAssetFiles::cleanup_staging(self)
    }

    fn quarantine_unknown_assets(
        &self,
        tracked_storage_relpaths: &std::collections::HashSet<String>,
    ) -> Result<u32, FoundationError> {
        EncryptedAssetFiles::quarantine_unknown_assets(self, tracked_storage_relpaths)
    }
}

struct NormalizedImage {
    source_format: ImageSourceFormat,
    pixel_width: u32,
    pixel_height: u32,
    sha256: [u8; 32],
    bytes: Vec<u8>,
}

fn normalize_image(input: ImageInput) -> Result<NormalizedImage, FoundationError> {
    if input.bytes.is_empty() || input.bytes.len() > MAX_IMAGE_INPUT_BYTES {
        return Err(FoundationError::image_too_large());
    }
    if input.source == ImageInputSource::ClipboardScreenshot
        && input.declared_format != ImageSourceFormat::Png
    {
        return Err(FoundationError::image_rejected());
    }

    let reader = ImageReader::new(Cursor::new(input.bytes.as_slice()))
        .with_guessed_format()
        .map_err(|_| FoundationError::image_rejected())?;
    if reader.format() != Some(image_format(input.declared_format)) {
        return Err(FoundationError::image_rejected());
    }
    let mut decoder = reader
        .into_decoder()
        .map_err(|_| FoundationError::image_rejected())?;
    let (width, height) = decoder.dimensions();
    validate_dimensions(width, height)?;

    let mut limits = Limits::default();
    limits.max_image_width = Some(MAX_IMAGE_DIMENSION);
    limits.max_image_height = Some(MAX_IMAGE_DIMENSION);
    limits.max_alloc = Some((MAX_IMAGE_PIXELS * 8).min(160 * 1024 * 1024));
    decoder
        .set_limits(limits)
        .map_err(|_| FoundationError::image_too_large())?;
    let orientation = decoder
        .orientation()
        .map_err(|_| FoundationError::image_rejected())?;
    let mut decoded =
        DynamicImage::from_decoder(decoder).map_err(|_| FoundationError::image_rejected())?;
    decoded.apply_orientation(orientation);
    let (pixel_width, pixel_height) = decoded.dimensions();
    validate_dimensions(pixel_width, pixel_height)?;

    let rgba = decoded.into_rgba8();
    let mut bytes = Vec::new();
    PngEncoder::new(&mut bytes)
        .write_image(
            rgba.as_raw(),
            pixel_width,
            pixel_height,
            image::ExtendedColorType::Rgba8,
        )
        .map_err(|_| FoundationError::image_rejected())?;
    if bytes.len() > MAX_NORMALIZED_IMAGE_BYTES {
        return Err(FoundationError::image_too_large());
    }
    let sha256 = Sha256::digest(&bytes).into();
    Ok(NormalizedImage {
        source_format: input.declared_format,
        pixel_width,
        pixel_height,
        sha256,
        bytes,
    })
}

fn image_format(format: ImageSourceFormat) -> ImageFormat {
    match format {
        ImageSourceFormat::Png => ImageFormat::Png,
        ImageSourceFormat::Jpeg => ImageFormat::Jpeg,
        ImageSourceFormat::WebP => ImageFormat::WebP,
    }
}

fn validate_normalized_png(bytes: &[u8]) -> Result<(), FoundationError> {
    normalized_png_dimensions(bytes).map(|_| ())
}

fn normalized_png_dimensions(bytes: &[u8]) -> Result<(u32, u32), FoundationError> {
    if bytes.len() > MAX_NORMALIZED_IMAGE_BYTES {
        return Err(FoundationError::asset_corrupted());
    }
    let reader = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|_| FoundationError::asset_corrupted())?;
    if reader.format() != Some(ImageFormat::Png) {
        return Err(FoundationError::asset_corrupted());
    }
    let (width, height) = reader
        .into_dimensions()
        .map_err(|_| FoundationError::asset_corrupted())?;
    validate_dimensions(width, height).map_err(|_| FoundationError::asset_corrupted())?;
    Ok((width, height))
}

fn validate_dimensions(width: u32, height: u32) -> Result<(), FoundationError> {
    let pixels = u64::from(width)
        .checked_mul(u64::from(height))
        .ok_or_else(FoundationError::image_too_large)?;
    if width == 0
        || height == 0
        || width > MAX_IMAGE_DIMENSION
        || height > MAX_IMAGE_DIMENSION
        || pixels > MAX_IMAGE_PIXELS
    {
        return Err(FoundationError::image_too_large());
    }
    Ok(())
}

#[cfg(test)]
fn encrypt_container(
    asset_key: &SecretKey,
    asset_id: &str,
    plaintext: &[u8],
) -> Result<Zeroizing<Vec<u8>>, FoundationError> {
    let mut output = Zeroizing::new(Vec::with_capacity(
        DNIMG_HEADER_LEN
            + plaintext.len()
            + plaintext.len().div_ceil(DNIMG_CHUNK_SIZE as usize) * DNIMG_TAG_LEN,
    ));
    write_encrypted_container(asset_key, asset_id, plaintext, &mut *output)?;
    Ok(output)
}

fn decrypt_container_file(
    asset_key: &SecretKey,
    asset_id: &str,
    path: &Path,
) -> Result<Zeroizing<Vec<u8>>, FoundationError> {
    validate_note_id(asset_id)?;
    let mut file = File::open(path).map_err(|error| match error.kind() {
        io::ErrorKind::NotFound => FoundationError::asset_not_found(),
        _ => map_asset_io(error),
    })?;
    let length = file.metadata().map_err(map_asset_io)?.len();
    if length > (DNIMG_HEADER_LEN + MAX_NORMALIZED_IMAGE_BYTES + 8192) as u64 {
        return Err(FoundationError::asset_corrupted());
    }
    let mut bytes = Vec::with_capacity(length as usize);
    file.read_to_end(&mut bytes).map_err(map_asset_io)?;
    decrypt_container(asset_key, asset_id, &bytes)
}

fn decrypt_container(
    asset_key: &SecretKey,
    asset_id: &str,
    container: &[u8],
) -> Result<Zeroizing<Vec<u8>>, FoundationError> {
    let header = decode_header(container)?;
    let plain_length =
        usize::try_from(header.plain_length).map_err(|_| FoundationError::asset_corrupted())?;
    if plain_length == 0 || plain_length > MAX_NORMALIZED_IMAGE_BYTES {
        return Err(FoundationError::asset_corrupted());
    }
    let chunk_size = DNIMG_CHUNK_SIZE as usize;
    let chunk_count = plain_length.div_ceil(chunk_size);
    let expected_length = DNIMG_HEADER_LEN
        .checked_add(plain_length)
        .and_then(|value| value.checked_add(chunk_count.checked_mul(DNIMG_TAG_LEN)?))
        .ok_or_else(FoundationError::asset_corrupted)?;
    if container.len() != expected_length {
        return Err(FoundationError::asset_corrupted());
    }

    let header_hash: [u8; 32] = Sha256::digest(&container[..DNIMG_HEADER_LEN]).into();
    let container_key = derive_container_key(asset_key, asset_id, &header.salt)?;
    let cipher = Aes256Gcm::new_from_slice(container_key.as_slice())
        .map_err(|_| asset_crypto_unavailable())?;
    let mut plaintext = Zeroizing::new(Vec::with_capacity(plain_length));
    let mut offset = DNIMG_HEADER_LEN;
    for index in 0..chunk_count {
        let remaining = plain_length - plaintext.len();
        let current_plain_length = remaining.min(chunk_size);
        let current_cipher_length = current_plain_length + DNIMG_TAG_LEN;
        let end = offset + current_cipher_length;
        let chunk_index = u32::try_from(index).map_err(|_| FoundationError::asset_corrupted())?;
        let nonce = chunk_nonce(&header.nonce_prefix, chunk_index);
        let aad = chunk_aad(
            &header_hash,
            asset_id,
            chunk_index,
            current_plain_length as u32,
            header.plain_length,
        );
        let decrypted = cipher
            .decrypt(
                &nonce,
                Payload {
                    msg: &container[offset..end],
                    aad: &aad,
                },
            )
            .map_err(|_| FoundationError::asset_corrupted())?;
        plaintext.extend_from_slice(&decrypted);
        offset = end;
    }
    if plaintext.len() != plain_length {
        return Err(FoundationError::asset_corrupted());
    }
    Ok(plaintext)
}

struct ContainerHeader {
    salt: [u8; DNIMG_SALT_LEN],
    nonce_prefix: [u8; DNIMG_NONCE_PREFIX_LEN],
    plain_length: u64,
}

fn encode_header(
    salt: &[u8; DNIMG_SALT_LEN],
    nonce_prefix: &[u8; DNIMG_NONCE_PREFIX_LEN],
    plain_length: u64,
) -> Vec<u8> {
    let mut output = Vec::with_capacity(DNIMG_HEADER_LEN);
    output.extend_from_slice(DNIMG_MAGIC);
    output.extend_from_slice(&DNIMG_FORMAT_VERSION.to_le_bytes());
    output.push(DNIMG_ALGORITHM_AES_256_GCM);
    output.extend_from_slice(DNIMG_KEY_ID);
    output.extend_from_slice(&DNIMG_CHUNK_SIZE.to_le_bytes());
    output.extend_from_slice(salt);
    output.extend_from_slice(nonce_prefix);
    output.extend_from_slice(&plain_length.to_le_bytes());
    output
}

fn decode_header(container: &[u8]) -> Result<ContainerHeader, FoundationError> {
    if container.len() < DNIMG_HEADER_LEN || &container[..8] != DNIMG_MAGIC {
        return Err(FoundationError::asset_corrupted());
    }
    let version = u16::from_le_bytes(container[8..10].try_into().expect("fixed header slice"));
    let algorithm = container[10];
    let key_id = &container[11..19];
    let chunk_size = u32::from_le_bytes(container[19..23].try_into().expect("fixed header slice"));
    if version != DNIMG_FORMAT_VERSION
        || algorithm != DNIMG_ALGORITHM_AES_256_GCM
        || key_id != DNIMG_KEY_ID
        || chunk_size != DNIMG_CHUNK_SIZE
    {
        return Err(FoundationError::asset_corrupted());
    }
    Ok(ContainerHeader {
        salt: container[23..39].try_into().expect("fixed header slice"),
        nonce_prefix: container[39..47].try_into().expect("fixed header slice"),
        plain_length: u64::from_le_bytes(container[47..55].try_into().expect("fixed header slice")),
    })
}

fn derive_container_key(
    asset_key: &SecretKey,
    asset_id: &str,
    salt: &[u8; DNIMG_SALT_LEN],
) -> Result<Zeroizing<[u8; 32]>, FoundationError> {
    let hkdf = Hkdf::<Sha256>::new(Some(salt), asset_key.as_bytes());
    let mut info = Vec::with_capacity(DNIMG_CONTAINER_KEY_INFO.len() + 1 + asset_id.len());
    info.extend_from_slice(DNIMG_CONTAINER_KEY_INFO);
    info.push(0);
    info.extend_from_slice(asset_id.as_bytes());
    let mut key = Zeroizing::new([0_u8; 32]);
    hkdf.expand(&info, key.as_mut())
        .map_err(|_| asset_crypto_unavailable())?;
    Ok(key)
}

fn chunk_nonce(
    prefix: &[u8; DNIMG_NONCE_PREFIX_LEN],
    index: u32,
) -> Nonce<aes_gcm::aead::consts::U12> {
    let mut bytes = [0_u8; 12];
    bytes[..DNIMG_NONCE_PREFIX_LEN].copy_from_slice(prefix);
    bytes[DNIMG_NONCE_PREFIX_LEN..].copy_from_slice(&index.to_be_bytes());
    Nonce::from(bytes)
}

fn chunk_aad(
    header_hash: &[u8; 32],
    asset_id: &str,
    chunk_index: u32,
    chunk_plain_length: u32,
    total_plain_length: u64,
) -> Vec<u8> {
    let mut aad = Vec::with_capacity(32 + asset_id.len() + 2 + 4 + 4 + 8);
    aad.extend_from_slice(header_hash);
    aad.extend_from_slice(asset_id.as_bytes());
    aad.extend_from_slice(&DNIMG_FORMAT_VERSION.to_le_bytes());
    aad.extend_from_slice(&chunk_index.to_le_bytes());
    aad.extend_from_slice(&chunk_plain_length.to_le_bytes());
    aad.extend_from_slice(&total_plain_length.to_le_bytes());
    aad
}

fn storage_relative_path(asset_id: &str) -> Result<String, FoundationError> {
    validate_note_id(asset_id)?;
    let bytes = asset_id.as_bytes();
    if !bytes[..4].iter().all(u8::is_ascii_hexdigit) {
        return Err(FoundationError::validation_failed());
    }
    Ok(format!(
        "assets/{}/{}/{}.dnimg",
        &asset_id[0..2],
        &asset_id[2..4],
        asset_id.to_ascii_lowercase()
    ))
}

fn is_asset_staging_name(name: &str) -> bool {
    let Some(without_extension) = name.strip_suffix(".part") else {
        return false;
    };
    let Some((asset_id, suffix)) = without_extension.rsplit_once('-') else {
        return false;
    };
    validate_note_id(asset_id).is_ok()
        && suffix.len() == 16
        && suffix.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn asset_crypto_unavailable() -> FoundationError {
    FoundationError::new(
        ErrorCode::InternalError,
        "The encrypted image operation could not be completed.",
    )
}

fn data_root_unavailable() -> FoundationError {
    FoundationError::new(
        ErrorCode::DataRootUnavailable,
        "The encrypted image storage directory is unavailable.",
    )
}

fn map_asset_io(error: io::Error) -> FoundationError {
    if error.kind() == io::ErrorKind::StorageFull || error.raw_os_error() == Some(112) {
        FoundationError::new(ErrorCode::DiskFull, "The disk is full.")
    } else {
        data_root_unavailable()
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
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn container_round_trip_is_bound_to_asset_id_and_key() {
        let key = SecretKey::from_bytes([0x51; 32]);
        let wrong = SecretKey::from_bytes([0x52; 32]);
        let asset_id = "f17109e1-b955-4f15-9f9f-99182e82b2d0";
        let plaintext = vec![0x41; DNIMG_CHUNK_SIZE as usize + 97];
        let encrypted = encrypt_container(&key, asset_id, &plaintext).unwrap();

        assert_ne!(encrypted.as_slice(), plaintext.as_slice());
        assert_eq!(
            decrypt_container(&key, asset_id, &encrypted)
                .unwrap()
                .as_slice(),
            plaintext
        );
        assert_eq!(
            decrypt_container(&key, "b5c12385-9df3-4375-bada-1fa0547288cb", &encrypted)
                .unwrap_err()
                .code(),
            ErrorCode::AssetCorrupted
        );
        assert_eq!(
            decrypt_container(&wrong, asset_id, &encrypted)
                .unwrap_err()
                .code(),
            ErrorCode::AssetCorrupted
        );
    }

    #[test]
    fn header_ciphertext_tag_and_truncation_fail_closed() {
        let key = SecretKey::from_bytes([0x61; 32]);
        let asset_id = "f17109e1-b955-4f15-9f9f-99182e82b2d0";
        let plaintext = vec![0x42; DNIMG_CHUNK_SIZE as usize + 10];
        let encrypted = encrypt_container(&key, asset_id, &plaintext).unwrap();
        for index in [0, DNIMG_HEADER_LEN + 12, encrypted.len() - 1] {
            let mut corrupted = encrypted.to_vec();
            corrupted[index] ^= 0x80;
            assert_eq!(
                decrypt_container(&key, asset_id, &corrupted)
                    .unwrap_err()
                    .code(),
                ErrorCode::AssetCorrupted
            );
        }
        assert_eq!(
            decrypt_container(&key, asset_id, &encrypted[..encrypted.len() - 7])
                .unwrap_err()
                .code(),
            ErrorCode::AssetCorrupted
        );
    }

    #[test]
    fn quarantine_copy_collision_preserves_both_existing_files() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "desktop-notes-quarantine-collision-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir_all(&root).unwrap();
        let source = root.join("source.dnimg");
        let destination = root.join("existing.orphan");
        fs::write(&source, b"source application ciphertext").unwrap();
        fs::write(&destination, b"existing quarantine evidence").unwrap();

        assert_eq!(
            quarantine_file_by_copy(&source, &destination)
                .unwrap_err()
                .code(),
            ErrorCode::DataRootUnavailable
        );
        assert_eq!(fs::read(&source).unwrap(), b"source application ciphertext");
        assert_eq!(
            fs::read(&destination).unwrap(),
            b"existing quarantine evidence"
        );
        fs::remove_dir_all(root).unwrap();
    }
}
