//! SQLCipher and cryptographic infrastructure adapters.

mod assets;
mod backup;
mod key_derivation;
mod logging;
mod migrations;
mod sqlcipher;
mod system;

pub use assets::{
    DNIMG_CHUNK_SIZE, DNIMG_FORMAT_VERSION, EncryptedAssetFiles, ImageInput, ImageInputSource,
    ImageSourceFormat, MAX_IMAGE_DIMENSION, MAX_IMAGE_INPUT_BYTES, MAX_IMAGE_PIXELS,
    MAX_NORMALIZED_IMAGE_BYTES, PersistedImage,
};
pub use backup::{
    AutomaticBackupManager, BackupClassification, BackupDiscovery, BackupHealth, BackupHealthState,
    BackupRunOutcome, BackupTrigger, DNBAK_FORMAT_VERSION, ValidatedBackup,
};
pub use key_derivation::HkdfKeyDeriver;
pub use logging::SafeJsonlLogger;
pub use migrations::{Migration, MigrationRegistry};
pub use sqlcipher::SqlCipherStore;
pub use system::{SessionDateUndoStore, SystemClock, UuidV4Generator};
