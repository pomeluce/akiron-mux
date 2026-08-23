use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use serde::{Serialize, de::DeserializeOwned};

use crate::{
    ClientError, Result,
    models::{BackendProfileState, ClientPreferences},
};

#[derive(Debug, Clone)]
pub struct ClientPaths {
    pub config_dir: PathBuf,
}

impl ClientPaths {
    pub fn new(config_dir: impl Into<PathBuf>) -> Self {
        Self { config_dir: config_dir.into() }
    }

    pub fn backend_profiles(&self) -> PathBuf {
        self.config_dir.join("backends.json")
    }

    pub fn preferences(&self) -> PathBuf {
        self.config_dir.join("desktop-preferences.json")
    }
}

#[derive(Debug, Clone)]
pub struct ClientStateStore {
    paths: ClientPaths,
}

impl ClientStateStore {
    pub fn new(paths: ClientPaths) -> Self {
        Self { paths }
    }

    pub fn load_backend_profiles(&self) -> Result<BackendProfileState> {
        load_or_default(&self.paths.backend_profiles())
    }

    pub fn save_backend_profiles(&self, state: &BackendProfileState) -> Result<()> {
        save_json_atomically(&self.paths.backend_profiles(), state)
    }

    pub fn load_preferences(&self) -> Result<ClientPreferences> {
        load_or_default(&self.paths.preferences())
    }

    pub fn save_preferences(&self, preferences: &ClientPreferences) -> Result<()> {
        save_json_atomically(&self.paths.preferences(), preferences)
    }
}

fn load_or_default<T: DeserializeOwned + Default>(path: &Path) -> Result<T> {
    if !path.exists() {
        return Ok(T::default());
    }
    let bytes = fs::read(path).map_err(ClientError::Storage)?;
    serde_json::from_slice(&bytes).map_err(ClientError::InvalidState)
}

pub fn save_json_atomically(path: &Path, value: &impl Serialize) -> Result<()> {
    let parent = path.parent().ok_or_else(|| ClientError::Validation("Client state path has no parent".into()))?;
    fs::create_dir_all(parent).map_err(ClientError::Storage)?;
    let bytes = serde_json::to_vec_pretty(value).map_err(ClientError::InvalidState)?;
    let temporary = path.with_extension(format!("json.{}.tmp", operation_id()));
    let result = (|| {
        let mut file = OpenOptions::new().create_new(true).write(true).open(&temporary).map_err(ClientError::Storage)?;
        file.write_all(&bytes).map_err(ClientError::Storage)?;
        file.sync_all().map_err(ClientError::Storage)?;
        fs::rename(&temporary, path).map_err(ClientError::Storage)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

pub fn operation_id() -> String {
    static SEQUENCE: AtomicU64 = AtomicU64::new(0);
    let sequence = SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let timestamp = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_nanos();
    format!("{timestamp:032x}{sequence:016x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_round_trips_without_secret_material() {
        let directory = tempfile::tempdir().unwrap();
        let store = ClientStateStore::new(ClientPaths::new(directory.path()));
        let mut state = BackendProfileState::default();
        state.profiles.push(crate::BackendProfile {
            id: "remote-1".into(),
            name: "Remote".into(),
            kind: crate::BackendKind::Remote,
            address: "https://example.com".into(),
            instance_id: Some("instance".into()),
            has_credential: true,
            requires_auth: false,
            capabilities: vec!["device-auth".into()],
        });
        store.save_backend_profiles(&state).unwrap();
        let raw = fs::read_to_string(store.paths.backend_profiles()).unwrap();
        assert!(!raw.contains("token"));
        assert_eq!(store.load_backend_profiles().unwrap(), state);
    }

    #[test]
    fn preferences_have_stable_defaults_and_round_trip() {
        let directory = tempfile::tempdir().unwrap();
        let store = ClientStateStore::new(ClientPaths::new(directory.path()));
        assert_eq!(store.load_preferences().unwrap(), ClientPreferences::default());
        let preferences = ClientPreferences {
            terminal_font_size: 16.0,
            ..ClientPreferences::default()
        };
        store.save_preferences(&preferences).unwrap();
        assert_eq!(store.load_preferences().unwrap(), preferences);
    }
}
