use std::time::Instant;

use serde::{Deserialize, Serialize};

use crate::{
    EncryptedStore, FoundationError, KeyDeriver, KeyProtector, SafeLogEvent, SafeLogKind,
    SafeLogger,
};

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DatabaseState {
    Fresh,
    Reopened,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FoundationStatus {
    pub ready: bool,
    pub view: &'static str,
    pub database_state: DatabaseState,
    pub schema_version: u32,
    pub encryption: &'static str,
}

pub struct FoundationService<'a> {
    keys: &'a dyn KeyProtector,
    deriver: &'a dyn KeyDeriver,
    store: &'a dyn EncryptedStore,
    logger: &'a dyn SafeLogger,
}

impl<'a> FoundationService<'a> {
    pub fn new(
        keys: &'a dyn KeyProtector,
        deriver: &'a dyn KeyDeriver,
        store: &'a dyn EncryptedStore,
        logger: &'a dyn SafeLogger,
    ) -> Self {
        Self {
            keys,
            deriver,
            store,
            logger,
        }
    }

    pub fn initialize(&self, operation_id: &str) -> Result<FoundationStatus, FoundationError> {
        let started = Instant::now();
        let result = self.initialize_inner();
        let duration_ms = started.elapsed().as_millis().try_into().unwrap_or(u64::MAX);

        let event = match &result {
            Ok(status) => SafeLogEvent {
                operation_id: operation_id.to_owned(),
                module: "foundation",
                kind: SafeLogKind::FoundationReady,
                error_code: None,
                duration_ms,
                schema_version: Some(status.schema_version),
                os_capability: Some("dpapi_current_user"),
            },
            Err(error) => SafeLogEvent {
                operation_id: operation_id.to_owned(),
                module: "foundation",
                kind: SafeLogKind::FoundationFailed,
                error_code: Some(error.code()),
                duration_ms,
                schema_version: None,
                os_capability: Some("dpapi_current_user"),
            },
        };
        self.logger.event(event);
        result
    }

    fn initialize_inner(&self) -> Result<FoundationStatus, FoundationError> {
        let database_existed = self.store.exists()?;
        let root = match self.keys.load_key()? {
            Some(root) => root,
            None if database_existed => return Err(FoundationError::key_unavailable()),
            None => self.keys.create_and_store_key()?,
        };
        let database_key = self.deriver.derive_database_key(&root)?;
        let database = self.store.open_or_initialize(&database_key)?;

        Ok(FoundationStatus {
            ready: true,
            view: "today_normal_empty",
            database_state: if database.newly_created {
                DatabaseState::Fresh
            } else {
                DatabaseState::Reopened
            },
            schema_version: database.schema_version,
            encryption: "sqlcipher",
        })
    }
}
