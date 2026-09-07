use std::sync::atomic::{AtomicBool, Ordering};

use desktop_notes_core::{
    DatabaseStatus, EncryptedStore, ErrorCode, FoundationError, FoundationService, KeyDeriver,
    KeyProtector, RootKey, SafeLogger, SecretKey,
};

struct MissingKey {
    create_called: AtomicBool,
}

impl KeyProtector for MissingKey {
    fn load_key(&self) -> Result<Option<RootKey>, FoundationError> {
        Ok(None)
    }

    fn create_and_store_key(&self) -> Result<RootKey, FoundationError> {
        self.create_called.store(true, Ordering::SeqCst);
        Ok(RootKey::from_bytes([7; 32]))
    }
}

struct Deriver;

impl KeyDeriver for Deriver {
    fn derive_database_key(&self, _root: &RootKey) -> Result<SecretKey, FoundationError> {
        Ok(SecretKey::from_bytes([9; 32]))
    }
}

struct ExistingDatabase {
    open_called: AtomicBool,
}

impl EncryptedStore for ExistingDatabase {
    fn exists(&self) -> Result<bool, FoundationError> {
        Ok(true)
    }

    fn open_or_initialize(&self, _key: &SecretKey) -> Result<DatabaseStatus, FoundationError> {
        self.open_called.store(true, Ordering::SeqCst);
        unreachable!("an existing database without its key must never be opened or replaced")
    }
}

struct NoopLogger;

impl SafeLogger for NoopLogger {
    fn event(&self, _event: desktop_notes_core::SafeLogEvent) {}
}

#[test]
fn existing_database_without_key_fails_closed_without_creating_a_replacement() {
    let keys = MissingKey {
        create_called: AtomicBool::new(false),
    };
    let store = ExistingDatabase {
        open_called: AtomicBool::new(false),
    };
    let service = FoundationService::new(&keys, &Deriver, &store, &NoopLogger);

    let error = service.initialize("test-operation").unwrap_err();

    assert_eq!(error.code(), ErrorCode::KeyUnavailable);
    assert!(!keys.create_called.load(Ordering::SeqCst));
    assert!(!store.open_called.load(Ordering::SeqCst));
}
