use crate::core::models::{
    validate_codex_provider_models, validate_profile, validate_provider, AppType, CodexCatalog, CodexModel, Profile, Provider, Source, OFFICIAL_CODEX_BASE_URL,
    OFFICIAL_CODEX_PROVIDER_ID,
};
use crate::db::Db;
use anyhow::Context;
use serde::Deserialize;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

const APP_DIRECTORY: &str = "akmux";
const LEGACY_APP_DIRECTORIES: [&str; 2] = ["akiron-mux", "ccswitch"];
static CONFIG_DIRECTORY: OnceLock<PathBuf> = OnceLock::new();
static DATA_DIRECTORY: OnceLock<PathBuf> = OnceLock::new();

/// Config directory for AkironMux: XDG_CONFIG_HOME on Linux, AppData on Windows,
/// Library/Application Support on macOS.
pub fn config_dir() -> PathBuf {
    CONFIG_DIRECTORY
        .get_or_init(|| {
            let base = dirs::config_dir().unwrap_or_else(|| PathBuf::from("."));
            let current = base.join(APP_DIRECTORY);
            for legacy in LEGACY_APP_DIRECTORIES {
                migrate_legacy_directory(&base.join(legacy), &current);
            }
            migrate_legacy_file(&current.join("ccswitch.db"), &current.join("akmux.db"));
            current
        })
        .clone()
}

/// Data directory for AkironMux (logs, runtime data).
#[allow(dead_code)]
pub fn data_dir() -> PathBuf {
    DATA_DIRECTORY
        .get_or_init(|| {
            let base = dirs::data_dir().unwrap_or_else(|| PathBuf::from("."));
            let current = base.join(APP_DIRECTORY);
            for legacy in LEGACY_APP_DIRECTORIES {
                migrate_legacy_directory(&base.join(legacy), &current);
            }
            current
        })
        .clone()
}

/// Config DB path.
pub fn db_path() -> PathBuf {
    config_dir().join("akmux.db")
}

/// System defaults path (for TOML overrides). Uses XDG config + /etc fallback.
pub fn defaults_path() -> Option<PathBuf> {
    let user = config_dir().join("defaults.toml");
    if user.exists() {
        return Some(user);
    }
    for system in ["/etc/akmux/defaults.toml", "/etc/akiron-mux/defaults.toml", "/etc/ccswitch/defaults.toml"] {
        let path = PathBuf::from(system);
        if path.exists() {
            return Some(path);
        }
    }
    None
}

fn default_config_path() -> PathBuf {
    let user = config_dir().join("defaults.toml");
    if user.exists() {
        return user;
    }
    for system in ["/etc/akmux/defaults.toml", "/etc/akiron-mux/defaults.toml", "/etc/ccswitch/defaults.toml"] {
        let path = PathBuf::from(system);
        if path.exists() {
            return path;
        }
    }
    PathBuf::from("/etc/akmux/defaults.toml")
}

fn migrate_legacy_directory(legacy: &Path, current: &Path) {
    if !legacy.is_dir() {
        return;
    }
    if let Err(error) = copy_missing_directory(legacy, current) {
        eprintln!("Warning: failed to migrate {} to {}: {error}", legacy.display(), current.display());
    }
}

fn copy_missing_directory(source: &Path, destination: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(destination)?;
    for entry in std::fs::read_dir(source)? {
        let entry = entry?;
        let source_path = entry.path();
        let destination_path = destination.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_missing_directory(&source_path, &destination_path)?;
        } else if !destination_path.exists() {
            std::fs::copy(source_path, destination_path)?;
        }
    }
    Ok(())
}

fn migrate_legacy_file(legacy: &Path, current: &Path) {
    if current.exists() || !legacy.is_file() {
        return;
    }
    if let Err(error) = std::fs::copy(legacy, current) {
        eprintln!("Warning: failed to migrate {} to {}: {error}", legacy.display(), current.display());
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct DefaultsFile {
    #[serde(default)]
    #[allow(dead_code)]
    version: u32,
    #[serde(default)]
    claude_providers: Vec<ProviderToml>,
    #[serde(default)]
    codex_providers: Vec<ProviderToml>,
}

#[derive(Debug, Deserialize)]
struct ProviderToml {
    id: String,
    name: String,
    api_url: String,
    api_key: String,
    #[serde(default)]
    codex_catalog: CodexCatalog,
    #[serde(default)]
    profiles: Vec<ProfileToml>,
    #[serde(default)]
    models: Vec<CodexModel>,
}

#[derive(Debug, Deserialize)]
struct ProfileToml {
    id: String,
    name: String,
    #[serde(default, alias = "reasoning_model")]
    opus: String,
    #[serde(default)]
    sonnet: String,
    #[serde(default, alias = "task_model")]
    haiku: String,
    #[serde(default)]
    subagent: String,
    #[serde(default)]
    default: bool,
}

pub struct ConfigManager {
    db: Db,
    system_claude_providers: Vec<Provider>,
    system_codex_providers: Vec<Provider>,
}

fn into_provider(p: ProviderToml, include_profiles: bool) -> Provider {
    Provider {
        id: p.id,
        name: p.name,
        api_url: p.api_url,
        api_key: p.api_key,
        codex_catalog: p.codex_catalog,
        profiles: if include_profiles {
            p.profiles
                .into_iter()
                .map(|pr| {
                    let sonnet = if pr.sonnet.is_empty() { pr.opus.clone() } else { pr.sonnet };
                    let subagent = if pr.subagent.is_empty() { pr.haiku.clone() } else { pr.subagent };
                    Profile {
                        id: pr.id,
                        name: pr.name,
                        opus: pr.opus,
                        sonnet,
                        haiku: pr.haiku,
                        subagent,
                        default: pr.default,
                        source: Source::System,
                    }
                })
                .collect()
        } else {
            vec![]
        },
        models: if include_profiles {
            vec![]
        } else {
            p.models
                .into_iter()
                .map(|mut model| {
                    model.source = Source::System;
                    model
                })
                .collect()
        },
        source: Source::System,
    }
}

fn official_codex_provider() -> Provider {
    Provider {
        id: OFFICIAL_CODEX_PROVIDER_ID.into(),
        name: "OpenAI".into(),
        api_url: OFFICIAL_CODEX_BASE_URL.into(),
        api_key: String::new(),
        codex_catalog: CodexCatalog::BuiltIn,
        profiles: Vec::new(),
        models: Vec::new(),
        source: Source::System,
    }
}

impl ConfigManager {
    pub fn new(db_path: &Path, defaults_path: Option<&Path>) -> Result<Self, anyhow::Error> {
        let db = Db::open(db_path).context("Failed to open akmux.db")?;

        let default_path = default_config_path();
        let defaults_path = defaults_path.unwrap_or_else(|| &default_path);
        let (system_claude_providers, mut system_codex_providers) = if defaults_path.exists() {
            let content = std::fs::read_to_string(defaults_path)?;
            let defaults: DefaultsFile = toml::from_str(&content)?;
            (
                defaults.claude_providers.into_iter().map(|p| into_provider(p, true)).collect(),
                defaults.codex_providers.into_iter().map(|p| into_provider(p, false)).collect(),
            )
        } else {
            (vec![], vec![])
        };
        system_codex_providers.insert(0, official_codex_provider());

        validate_default_provider_ids("Claude", &system_claude_providers)?;
        validate_default_provider_ids("Codex", &system_codex_providers)?;
        for provider in system_claude_providers.iter().chain(&system_codex_providers) {
            validate_provider(provider).with_context(|| format!("Invalid provider '{}' in defaults.toml", provider.id))?;
            for profile in &provider.profiles {
                validate_profile(profile).with_context(|| format!("Invalid profile '{}/{}' in defaults.toml", provider.id, profile.id))?;
            }
            if !provider.models.is_empty() {
                validate_codex_provider_models(provider).with_context(|| format!("Invalid Codex models for '{}' in defaults.toml", provider.id))?;
            }
        }

        // Sync TOML providers/profiles to DB (source='system').
        // Always call — even when empty, to demote stale system providers.
        db.sync_system_providers("claude", &system_claude_providers)
            .context("Failed to sync Claude defaults to DB")?;
        db.sync_system_providers("codex", &system_codex_providers).context("Failed to sync Codex defaults to DB")?;
        db.sync_system_codex_models(&system_codex_providers).context("Failed to sync Codex models to DB")?;

        Ok(ConfigManager {
            db,
            system_claude_providers,
            system_codex_providers,
        })
    }

    pub(crate) fn db(&self) -> &Db {
        &self.db
    }
    pub fn get_setting(&self, key: &str) -> Option<String> {
        self.db.get_setting(key)
    }
    pub fn set_setting(&self, key: &str, value: &str) -> Result<(), rusqlite::Error> {
        self.db.set_setting(key, value)
    }

    pub fn list_providers(&self) -> Result<Vec<Provider>, anyhow::Error> {
        self.list_providers_for(AppType::Claude)
    }

    pub fn list_providers_for(&self, app: AppType) -> Result<Vec<Provider>, anyhow::Error> {
        let db_providers = self.db.get_providers(app.as_str())?;
        let mut result = match app {
            AppType::Claude => self.system_claude_providers.clone(),
            AppType::Codex => self.system_codex_providers.clone(),
        };

        for dp in &db_providers {
            if app == AppType::Codex && dp.id == OFFICIAL_CODEX_PROVIDER_ID {
                continue;
            }
            if let Some(existing) = result.iter_mut().find(|p| p.id == dp.id) {
                existing.name = dp.name.clone();
                existing.api_url = dp.api_url.clone();
                existing.api_key = dp.api_key.clone();
                existing.codex_catalog = dp.codex_catalog;
                existing.source = dp.source; // Use DB source (system/user)
            } else {
                result.push(dp.clone());
            }
        }

        if app == AppType::Codex {
            for provider in &mut result {
                provider.profiles.clear();
                let db_models = self.db.get_codex_models(&provider.id)?;
                for model in db_models {
                    if let Some(existing) = provider.models.iter_mut().find(|item| item.slug == model.slug) {
                        *existing = model;
                    } else {
                        provider.models.push(model);
                    }
                }
            }
            return Ok(result);
        }

        for provider in &mut result {
            let db_profiles = self.db.get_profiles(&provider.id)?;
            for dp in &db_profiles {
                if let Some(existing) = provider.profiles.iter_mut().find(|p| p.id == dp.id) {
                    *existing = dp.clone();
                } else {
                    provider.profiles.push(dp.clone());
                }
            }
        }
        Ok(result)
    }

    pub fn find_profile(&self, provider_id: &str, profile_id: &str) -> Result<Option<(Provider, Profile)>, anyhow::Error> {
        for p in self.list_providers()? {
            if p.id == provider_id {
                for pr in &p.profiles {
                    if pr.id == profile_id {
                        return Ok(Some((p.clone(), pr.clone())));
                    }
                }
            }
        }
        Ok(None)
    }

    pub fn find_provider_for(&self, app: AppType, provider_id: &str) -> Result<Option<Provider>, anyhow::Error> {
        Ok(self.list_providers_for(app)?.into_iter().find(|provider| provider.id == provider_id))
    }
}

fn validate_default_provider_ids(app: &str, providers: &[Provider]) -> anyhow::Result<()> {
    let mut provider_ids = std::collections::HashSet::new();
    for provider in providers {
        if !provider_ids.insert(provider.id.as_str()) {
            anyhow::bail!("Duplicate {} provider ID '{}' in defaults.toml", app, provider.id);
        }
        let mut profile_ids = std::collections::HashSet::new();
        for profile in &provider.profiles {
            if !profile_ids.insert(profile.id.as_str()) {
                anyhow::bail!("Duplicate profile ID '{}/{}' in defaults.toml", provider.id, profile.id);
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod migration_tests {
    use super::*;

    #[test]
    fn migration_copies_missing_files_without_overwriting_new_data() {
        let directory = tempfile::tempdir().unwrap();
        let legacy = directory.path().join("ccswitch");
        let current = directory.path().join("akmux");
        std::fs::create_dir_all(legacy.join("nested")).unwrap();
        std::fs::create_dir_all(&current).unwrap();
        std::fs::write(legacy.join("ccswitch.db"), "legacy-db").unwrap();
        std::fs::write(legacy.join("nested/settings.toml"), "legacy-settings").unwrap();
        std::fs::write(current.join("ccswitch.db"), "current-db").unwrap();

        migrate_legacy_directory(&legacy, &current);

        assert_eq!(std::fs::read_to_string(current.join("ccswitch.db")).unwrap(), "current-db");
        assert_eq!(std::fs::read_to_string(current.join("nested/settings.toml")).unwrap(), "legacy-settings");
    }

    #[test]
    fn legacy_database_is_copied_to_the_new_filename() {
        let directory = tempfile::tempdir().unwrap();
        let legacy = directory.path().join("ccswitch.db");
        let current = directory.path().join("akmux.db");
        std::fs::write(&legacy, "database").unwrap();

        migrate_legacy_file(&legacy, &current);

        assert_eq!(std::fs::read_to_string(current).unwrap(), "database");
        assert!(legacy.exists());
    }
}
