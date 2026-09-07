use desktop_notes_core::{ErrorCode, FoundationError, KeyDeriver, RootKey, SecretKey};
use hkdf::Hkdf;
use sha2::Sha256;
use zeroize::Zeroizing;

const DATABASE_KEY_INFO: &[u8] = b"db-v1";
const ASSET_KEY_INFO: &[u8] = b"assets-v1";
const BACKUP_KEY_INFO: &[u8] = b"backup-v1";

#[derive(Default)]
pub struct HkdfKeyDeriver;

impl KeyDeriver for HkdfKeyDeriver {
    fn derive_database_key(&self, root: &RootKey) -> Result<SecretKey, FoundationError> {
        let hkdf = Hkdf::<Sha256>::new(None, root.as_bytes());
        let mut database_key = Zeroizing::new([0_u8; 32]);
        hkdf.expand(DATABASE_KEY_INFO, database_key.as_mut())
            .map_err(|_| {
                FoundationError::new(
                    ErrorCode::InternalError,
                    "The database key could not be derived.",
                )
            })?;
        Ok(SecretKey::from_bytes(*database_key))
    }
}

impl HkdfKeyDeriver {
    pub fn derive_asset_key(&self, root: &RootKey) -> Result<SecretKey, FoundationError> {
        let hkdf = Hkdf::<Sha256>::new(None, root.as_bytes());
        let mut asset_key = Zeroizing::new([0_u8; 32]);
        hkdf.expand(ASSET_KEY_INFO, asset_key.as_mut())
            .map_err(|_| {
                FoundationError::new(
                    ErrorCode::InternalError,
                    "The asset key could not be derived.",
                )
            })?;
        Ok(SecretKey::from_bytes(*asset_key))
    }

    pub fn derive_backup_key(&self, root: &RootKey) -> Result<SecretKey, FoundationError> {
        derive_domain_key(
            root,
            BACKUP_KEY_INFO,
            "The backup key could not be derived.",
        )
    }
}

fn derive_domain_key(
    root: &RootKey,
    info: &[u8],
    message: &'static str,
) -> Result<SecretKey, FoundationError> {
    let hkdf = Hkdf::<Sha256>::new(None, root.as_bytes());
    let mut key = Zeroizing::new([0_u8; 32]);
    hkdf.expand(info, key.as_mut())
        .map_err(|_| FoundationError::new(ErrorCode::InternalError, message))?;
    Ok(SecretKey::from_bytes(*key))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn asset_key_is_deterministic_and_domain_separated_from_database_key() {
        let root = RootKey::from_bytes([0x39; 32]);
        let deriver = HkdfKeyDeriver;

        let first = deriver.derive_asset_key(&root).unwrap();
        let second = deriver.derive_asset_key(&root).unwrap();
        let database = deriver.derive_database_key(&root).unwrap();

        assert_eq!(first.as_bytes(), second.as_bytes());
        assert_ne!(first.as_bytes(), database.as_bytes());
        let backup = deriver.derive_backup_key(&root).unwrap();
        assert_ne!(first.as_bytes(), backup.as_bytes());
        assert_ne!(database.as_bytes(), backup.as_bytes());
    }
}
