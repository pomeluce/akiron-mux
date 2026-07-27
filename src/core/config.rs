use crate::core::models::{
    validate_profile, validate_provider, AppType, Profile, Provider, Source,
};
use crate::db::Db;
use anyhow::Context;
use serde::Deserialize;
use std::path::{Path, PathBuf};

/// Config directory for ccswitch: XDG_CONFIG_HOME on Linux, AppData on Windows,
/// Library/Application Support on macOS.
pub fn config_dir() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("ccswitch")
}

/// Data directory for ccswitch (logs, runtime data).
#[allow(dead_code)]
pub fn data_dir() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("ccswitch")
}

/// Config DB path.
pub fn db_path() -> PathBuf {
    config_dir().join("ccswitch.db")
}

/// System defaults path (for TOML overrides). Uses XDG config + /etc fallback.
pub fn defaults_path() -> Option<PathBuf> {
    let user = config_dir().join("defaults.toml");
    if user.exists() {
        return Some(user);
    }
    let system = PathBuf::from("/etc/ccswitch/defaults.toml");
    if system.exists() {
        return Some(system);
    }
    None
}

fn default_config_path() -> PathBuf {
    let user = config_dir().join("defaults.toml");
    if user.exists() {
        return user;
    }
    PathBuf::from("/etc/ccswitch/defaults.toml")
}

#[derive(Debug, Deserialize)]
struct DefaultsFile {
    #[serde(default)]
    #[allow(dead_code)]
    version: u32,
    #[serde(default)]
    providers: Vec<ProviderToml>,
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
    profiles: Vec<ProfileToml>,
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
        profiles: if include_profiles {
            p.profiles
                .into_iter()
                .map(|pr| {
                    let sonnet = if pr.sonnet.is_empty() {
                        pr.opus.clone()
                    } else {
                        pr.sonnet
                    };
                    let subagent = if pr.subagent.is_empty() {
                        pr.haiku.clone()
                    } else {
                        pr.subagent
                    };
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
        source: Source::System,
    }
}

impl ConfigManager {
    pub fn new(db_path: &Path, defaults_path: Option<&Path>) -> Result<Self, anyhow::Error> {
        let db = Db::open(db_path).context("Failed to open ccswitch.db")?;

        let default_path = default_config_path();
        let defaults_path = defaults_path.unwrap_or_else(|| &default_path);
        let (system_claude_providers, system_codex_providers) = if defaults_path.exists() {
            let content = std::fs::read_to_string(defaults_path)?;
            let defaults: DefaultsFile = toml::from_str(&content)?;
            (
                defaults
                    .providers
                    .into_iter()
                    .map(|p| into_provider(p, true))
                    .collect(),
                defaults
                    .codex_providers
                    .into_iter()
                    .map(|p| into_provider(p, false))
                    .collect(),
            )
        } else {
            (vec![], vec![])
        };

        validate_default_provider_ids("Claude", &system_claude_providers)?;
        validate_default_provider_ids("Codex", &system_codex_providers)?;
        for provider in system_claude_providers
            .iter()
            .chain(&system_codex_providers)
        {
            validate_provider(provider)
                .with_context(|| format!("Invalid provider '{}' in defaults.toml", provider.id))?;
            for profile in &provider.profiles {
                validate_profile(profile).with_context(|| {
                    format!(
                        "Invalid profile '{}/{}' in defaults.toml",
                        provider.id, profile.id
                    )
                })?;
            }
        }

        // Sync TOML providers/profiles to DB (source='system').
        // Always call — even when empty, to demote stale system providers.
        db.sync_system_providers("claude", &system_claude_providers)
            .context("Failed to sync Claude defaults to DB")?;
        db.sync_system_providers("codex", &system_codex_providers)
            .context("Failed to sync Codex defaults to DB")?;

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
            if let Some(existing) = result.iter_mut().find(|p| p.id == dp.id) {
                existing.name = dp.name.clone();
                existing.api_url = dp.api_url.clone();
                existing.api_key = dp.api_key.clone();
                existing.source = dp.source; // Use DB source (system/user)
            } else {
                result.push(dp.clone());
            }
        }

        if app == AppType::Codex {
            for provider in &mut result {
                provider.profiles.clear();
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

    pub fn find_profile(
        &self,
        provider_id: &str,
        profile_id: &str,
    ) -> Result<Option<(Provider, Profile)>, anyhow::Error> {
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

    pub fn find_provider_for(
        &self,
        app: AppType,
        provider_id: &str,
    ) -> Result<Option<Provider>, anyhow::Error> {
        Ok(self
            .list_providers_for(app)?
            .into_iter()
            .find(|provider| provider.id == provider_id))
    }
}

fn validate_default_provider_ids(app: &str, providers: &[Provider]) -> anyhow::Result<()> {
    let mut provider_ids = std::collections::HashSet::new();
    for provider in providers {
        if !provider_ids.insert(provider.id.as_str()) {
            anyhow::bail!(
                "Duplicate {} provider ID '{}' in defaults.toml",
                app,
                provider.id
            );
        }
        let mut profile_ids = std::collections::HashSet::new();
        for profile in &provider.profiles {
            if !profile_ids.insert(profile.id.as_str()) {
                anyhow::bail!(
                    "Duplicate profile ID '{}/{}' in defaults.toml",
                    provider.id,
                    profile.id
                );
            }
        }
    }
    Ok(())
}
