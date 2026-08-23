use std::{collections::HashMap, sync::Mutex};

use zeroize::Zeroizing;

use crate::{ClientError, Result};

pub trait CredentialStore: Send + Sync {
    fn store(&self, profile_id: &str, token: &str) -> Result<()>;
    fn load(&self, profile_id: &str) -> Result<Zeroizing<String>>;
    fn delete(&self, profile_id: &str) -> Result<()>;
}

#[derive(Default)]
pub struct MemoryCredentialStore(Mutex<HashMap<String, Zeroizing<String>>>);

impl CredentialStore for MemoryCredentialStore {
    fn store(&self, profile_id: &str, token: &str) -> Result<()> {
        self.0
            .lock()
            .map_err(|_| ClientError::Platform("Credential memory is unavailable".into()))?
            .insert(profile_id.into(), Zeroizing::new(token.into()));
        Ok(())
    }

    fn load(&self, profile_id: &str) -> Result<Zeroizing<String>> {
        self.0
            .lock()
            .map_err(|_| ClientError::Platform("Credential memory is unavailable".into()))?
            .get(profile_id)
            .cloned()
            .ok_or_else(|| ClientError::Authentication("Backend credential is unavailable; re-authentication is required".into()))
    }

    fn delete(&self, profile_id: &str) -> Result<()> {
        self.0
            .lock()
            .map_err(|_| ClientError::Platform("Credential memory is unavailable".into()))?
            .remove(profile_id);
        Ok(())
    }
}
