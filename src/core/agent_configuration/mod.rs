mod native;
mod reconcile;

use crate::core::codex_catalog::{build_catalog, write_catalog};
use crate::core::config::ConfigManager;
use crate::core::models::{validate_codex_model, validate_profile, validate_provider, ActiveConfig, AppType, CodexModel, Profile, Provider, SwitchMode};
use anyhow::{anyhow, Context};
use rusqlite::{params, Transaction, TransactionBehavior};
use std::path::{Path, PathBuf};

const DEFAULT_PROXY_PORT: &str = "15721";

#[derive(Clone, Debug)]
pub struct AgentConfigPaths {
    claude_settings: PathBuf,
    codex_config: PathBuf,
    codex_auth: PathBuf,
}

impl AgentConfigPaths {
    pub fn new(claude_settings: PathBuf, codex_config: PathBuf, codex_auth: PathBuf) -> Self {
        Self {
            claude_settings,
            codex_config,
            codex_auth,
        }
    }

    fn codex_catalog(&self) -> PathBuf {
        self.codex_config
            .parent()
            .map(|directory| directory.join("akmux/models.json"))
            .unwrap_or_else(|| PathBuf::from("akmux/models.json"))
    }

    fn codex_native_files(&self) -> Vec<PathBuf> {
        vec![
            self.codex_config.clone(),
            self.codex_auth.clone(),
            self.codex_auth.with_file_name("auth_openai.json"),
            self.codex_auth.with_file_name("auth_akmux.json"),
            self.codex_catalog(),
        ]
    }
}

impl Default for AgentConfigPaths {
    fn default() -> Self {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
        let home = Path::new(&home);
        Self::new(home.join(".claude/settings.json"), home.join(".codex/config.toml"), home.join(".codex/auth.json"))
    }
}

#[derive(Debug, thiserror::Error)]
pub enum AgentConfigurationError {
    #[error("Cannot delete active {kind} '{selection}'; switch to another configuration first")]
    ActiveSelection { kind: &'static str, selection: String },
    #[error("{action} failed: {source:#}")]
    NotApplied {
        action: &'static str,
        #[source]
        source: anyhow::Error,
    },
    #[error("{action} left the external Agent state uncertain: {source:#}")]
    ExternalStateUncertain {
        action: &'static str,
        #[source]
        source: anyhow::Error,
    },
    #[error("Configuration Catalog was saved, but the Codex model catalog is pending synchronization: {source:#}")]
    DerivedArtifactPending {
        #[source]
        source: anyhow::Error,
    },
}

pub type Result<T> = std::result::Result<T, AgentConfigurationError>;

pub struct AgentConfiguration<'a> {
    mgr: &'a ConfigManager,
    paths: AgentConfigPaths,
}

impl<'a> AgentConfiguration<'a> {
    pub fn new(mgr: &'a ConfigManager) -> Self {
        Self::with_paths(mgr, AgentConfigPaths::default())
    }

    pub fn with_paths(mgr: &'a ConfigManager, paths: AgentConfigPaths) -> Self {
        Self { mgr, paths }
    }

    pub fn reconcile(&self) -> Result<()> {
        let mut failures = Vec::new();
        if let Err(error) = reconcile::reconcile_claude(self.mgr, &self.paths.claude_settings) {
            failures.push(format!("Claude: {error:#}"));
        }
        if let Err(error) = reconcile::reconcile_codex(self.mgr, &self.paths.codex_config) {
            failures.push(format!("Codex: {error:#}"));
        }
        if let Err(error) = self.rebuild_codex_catalog_if_present() {
            failures.push(format!("Codex model catalog: {error:#}"));
        }
        if failures.is_empty() {
            Ok(())
        } else {
            Err(not_applied("Reconcile Agent configuration", anyhow!(failures.join("; "))))
        }
    }

    pub fn apply_claude_profile(&self, provider_id: &str, profile_id: &str, mode: SwitchMode) -> Result<ActiveConfig> {
        self.native_transaction("Apply Claude profile", [self.paths.claude_settings.as_path()], |transaction| {
            let config = native::apply_claude_profile(self.mgr, provider_id, profile_id, mode, &self.paths.claude_settings)?;
            let proxy_mode = (mode == SwitchMode::Proxy).to_string();
            set_settings_in(
                transaction,
                &[
                    ("active_provider", config.provider_id.as_str()),
                    ("active_profile", config.profile_id.as_str()),
                    ("proxy_mode", proxy_mode.as_str()),
                ],
            )?;
            if mode == SwitchMode::Proxy {
                set_settings_in(transaction, &[("proxy_port", DEFAULT_PROXY_PORT)])?;
            }
            Ok(config)
        })
    }

    pub fn apply_codex_provider(&self, provider_id: &str) -> Result<Provider> {
        let provider = self
            .mgr
            .find_provider_for(AppType::Codex, provider_id)
            .map_err(|source| not_applied("Apply Codex provider", source))?
            .ok_or_else(|| not_applied("Apply Codex provider", anyhow!("Codex provider not found: {provider_id}")))?;
        let model = provider
            .models
            .iter()
            .find(|model| model.default)
            .or_else(|| provider.models.first())
            .map(|model| model.slug.as_str());
        self.apply_codex_model(provider_id, model)
    }

    pub fn apply_codex_model(&self, provider_id: &str, model_slug: Option<&str>) -> Result<Provider> {
        let native_files = self.paths.codex_native_files();
        self.native_transaction("Apply Codex model", native_files.iter().map(PathBuf::as_path), |transaction| {
            let provider = native::apply_codex_model(self.mgr, provider_id, model_slug, &self.paths.codex_config, &self.paths.codex_auth)?;
            set_settings_in(
                transaction,
                &[
                    (AppType::Codex.active_provider_key(), provider.id.as_str()),
                    ("active_codex_model", model_slug.unwrap_or("")),
                ],
            )?;
            Ok(provider)
        })
    }

    pub fn save_provider(&self, app: AppType, provider: &Provider) -> Result<()> {
        validate_provider(provider).map_err(|source| not_applied("Save provider", source))?;
        if app == AppType::Codex {
            let mut prospective = self.mgr.list_providers_for(app).map_err(|source| not_applied("Save provider", source))?;
            if let Some(existing) = prospective.iter_mut().find(|item| item.id == provider.id) {
                existing.name.clone_from(&provider.name);
                existing.api_url.clone_from(&provider.api_url);
                existing.api_key.clone_from(&provider.api_key);
                existing.codex_catalog = provider.codex_catalog;
            } else {
                prospective.push(provider.clone());
            }
            build_catalog(&prospective).map_err(|source| not_applied("Save provider", source))?;
        }
        let active = self.mgr.get_setting(app.active_provider_key()).as_deref() == Some(provider.id.as_str());
        if active {
            let native_files = match app {
                AppType::Claude => vec![self.paths.claude_settings.clone()],
                AppType::Codex => self.paths.codex_native_files(),
            };
            return self.native_transaction("Save provider", native_files.iter().map(PathBuf::as_path), |transaction| {
                self.mgr.db().save_provider_in(transaction, provider, app.as_str())?;
                if app == AppType::Codex {
                    self.validate_codex_catalog()?;
                }
                match app {
                    AppType::Claude => {
                        let profile = self.mgr.get_setting("active_profile").unwrap_or_default();
                        let mode = if self.mgr.get_setting("proxy_mode").as_deref() == Some("true") {
                            SwitchMode::Proxy
                        } else {
                            SwitchMode::Local
                        };
                        native::apply_claude_profile(self.mgr, &provider.id, &profile, mode, &self.paths.claude_settings)?;
                        let proxy_mode = (mode == SwitchMode::Proxy).to_string();
                        set_settings_in(
                            transaction,
                            &[
                                ("active_provider", provider.id.as_str()),
                                ("active_profile", profile.as_str()),
                                ("proxy_mode", proxy_mode.as_str()),
                            ],
                        )?;
                    }
                    AppType::Codex => {
                        let model = self.mgr.get_setting("active_codex_model").unwrap_or_default();
                        native::apply_codex_model(
                            self.mgr,
                            &provider.id,
                            (!model.is_empty()).then_some(model.as_str()),
                            &self.paths.codex_config,
                            &self.paths.codex_auth,
                        )?;
                        set_settings_in(
                            transaction,
                            &[(AppType::Codex.active_provider_key(), provider.id.as_str()), ("active_codex_model", model.as_str())],
                        )?;
                    }
                }
                Ok(())
            });
        }
        self.catalog_transaction("Save provider", |transaction| {
            self.mgr.db().save_provider_in(transaction, provider, app.as_str())?;
            if app == AppType::Codex {
                self.validate_codex_catalog()?;
            }
            Ok(())
        })?;
        if app == AppType::Codex {
            self.rebuild_codex_catalog_if_present()?;
        }
        Ok(())
    }

    pub fn delete_provider(&self, app: AppType, provider_id: &str) -> Result<()> {
        let provider = self
            .mgr
            .find_provider_for(app, provider_id)
            .map_err(|source| not_applied("Delete provider", source))?
            .ok_or_else(|| not_applied("Delete provider", anyhow!("Provider not found: {provider_id}")))?;
        if !provider.source.can_delete() {
            return Err(not_applied("Delete provider", anyhow!("Cannot delete system default provider '{provider_id}'")));
        }
        if self.mgr.get_setting(app.active_provider_key()).as_deref() == Some(provider_id) {
            return Err(AgentConfigurationError::ActiveSelection {
                kind: "provider",
                selection: provider_id.to_string(),
            });
        }
        self.catalog_transaction("Delete provider", |transaction| {
            self.mgr.db().delete_provider_in(transaction, provider_id, app.as_str())?;
            Ok(())
        })?;
        if app == AppType::Codex {
            self.rebuild_codex_catalog_if_present()?;
        }
        Ok(())
    }

    pub fn save_profile(&self, provider_id: &str, profile: &Profile) -> Result<()> {
        validate_profile(profile).map_err(|source| not_applied("Save model profile", source))?;
        if self
            .mgr
            .find_provider_for(AppType::Claude, provider_id)
            .map_err(|source| not_applied("Save model profile", source))?
            .is_none()
        {
            return Err(not_applied("Save model profile", anyhow!("Provider not found: {provider_id}")));
        }
        let active = self.mgr.get_setting("active_provider").as_deref() == Some(provider_id) && self.mgr.get_setting("active_profile").as_deref() == Some(profile.id.as_str());
        if active {
            let mode = if self.mgr.get_setting("proxy_mode").as_deref() == Some("true") {
                SwitchMode::Proxy
            } else {
                SwitchMode::Local
            };
            return self.native_transaction("Save model profile", [self.paths.claude_settings.as_path()], |transaction| {
                self.mgr.db().save_profile_in(transaction, provider_id, profile)?;
                native::apply_claude_profile(self.mgr, provider_id, &profile.id, mode, &self.paths.claude_settings)?;
                let proxy_mode = (mode == SwitchMode::Proxy).to_string();
                set_settings_in(
                    transaction,
                    &[
                        ("active_provider", provider_id),
                        ("active_profile", profile.id.as_str()),
                        ("proxy_mode", proxy_mode.as_str()),
                    ],
                )?;
                Ok(())
            });
        }
        self.catalog_transaction("Save model profile", |transaction| {
            self.mgr.db().save_profile_in(transaction, provider_id, profile)?;
            Ok(())
        })
    }

    pub fn delete_profile(&self, provider_id: &str, profile_id: &str) -> Result<()> {
        let (_, profile) = self
            .mgr
            .find_profile(provider_id, profile_id)
            .map_err(|source| not_applied("Delete model profile", source))?
            .ok_or_else(|| not_applied("Delete model profile", anyhow!("Model profile not found: {provider_id}/{profile_id}")))?;
        if !profile.source.can_delete() {
            return Err(not_applied(
                "Delete model profile",
                anyhow!("Cannot delete system default profile '{provider_id}/{profile_id}'"),
            ));
        }
        if self.mgr.get_setting("active_provider").as_deref() == Some(provider_id) && self.mgr.get_setting("active_profile").as_deref() == Some(profile_id) {
            return Err(AgentConfigurationError::ActiveSelection {
                kind: "model profile",
                selection: format!("{provider_id}/{profile_id}"),
            });
        }
        self.catalog_transaction("Delete model profile", |transaction| {
            self.mgr.db().delete_profile_in(transaction, provider_id, profile_id)?;
            Ok(())
        })
    }

    pub fn save_codex_model(&self, provider_id: &str, model: &CodexModel) -> Result<()> {
        validate_codex_model(model).map_err(|source| not_applied("Save Codex model", source))?;
        let mut prospective = self.mgr.list_providers_for(AppType::Codex).map_err(|source| not_applied("Save Codex model", source))?;
        let provider = prospective
            .iter_mut()
            .find(|provider| provider.id == provider_id)
            .ok_or_else(|| not_applied("Save Codex model", anyhow!("Codex provider not found: {provider_id}")))?;
        if model.default {
            for existing in &mut provider.models {
                existing.default = false;
            }
        }
        if let Some(existing) = provider.models.iter_mut().find(|existing| existing.slug == model.slug) {
            *existing = model.clone();
        } else {
            provider.models.push(model.clone());
        }
        build_catalog(&prospective).map_err(|source| not_applied("Save Codex model", source))?;
        let active = self.mgr.get_setting(AppType::Codex.active_provider_key()).as_deref() == Some(provider_id)
            && self.mgr.get_setting("active_codex_model").as_deref() == Some(model.slug.as_str());
        if active {
            let native_files = self.paths.codex_native_files();
            return self.native_transaction("Save Codex model", native_files.iter().map(PathBuf::as_path), |transaction| {
                self.mgr.db().insert_codex_model_in(transaction, provider_id, model)?;
                self.validate_codex_catalog()?;
                native::apply_codex_model(self.mgr, provider_id, Some(&model.slug), &self.paths.codex_config, &self.paths.codex_auth)?;
                set_settings_in(
                    transaction,
                    &[(AppType::Codex.active_provider_key(), provider_id), ("active_codex_model", model.slug.as_str())],
                )?;
                Ok(())
            });
        }
        self.catalog_transaction("Save Codex model", |transaction| {
            self.mgr.db().insert_codex_model_in(transaction, provider_id, model)?;
            self.validate_codex_catalog()?;
            Ok(())
        })?;
        self.rebuild_codex_catalog_if_present()
    }

    pub fn delete_codex_model(&self, provider_id: &str, model_slug: &str) -> Result<()> {
        let model = self
            .mgr
            .find_provider_for(AppType::Codex, provider_id)
            .map_err(|source| not_applied("Delete Codex model", source))?
            .and_then(|provider| provider.models.into_iter().find(|model| model.slug == model_slug))
            .ok_or_else(|| not_applied("Delete Codex model", anyhow!("Codex model not found: {provider_id}/{model_slug}")))?;
        if !model.source.can_delete() {
            return Err(not_applied(
                "Delete Codex model",
                anyhow!("Cannot delete system default Codex model '{provider_id}/{model_slug}'"),
            ));
        }
        if self.mgr.get_setting(AppType::Codex.active_provider_key()).as_deref() == Some(provider_id) && self.mgr.get_setting("active_codex_model").as_deref() == Some(model_slug) {
            return Err(AgentConfigurationError::ActiveSelection {
                kind: "Codex model",
                selection: format!("{provider_id}/{model_slug}"),
            });
        }
        self.catalog_transaction("Delete Codex model", |transaction| {
            self.mgr.db().clear_codex_model_in(transaction, provider_id, model_slug)?;
            Ok(())
        })?;
        self.rebuild_codex_catalog_if_present()
    }

    fn rebuild_codex_catalog_if_present(&self) -> Result<()> {
        let path = self.paths.codex_catalog();
        if !path.exists() {
            return Ok(());
        }
        let transaction = Transaction::new_unchecked(self.mgr.db().conn(), TransactionBehavior::Immediate)
            .map_err(|source| AgentConfigurationError::DerivedArtifactPending { source: source.into() })?;
        let providers = self
            .mgr
            .list_providers_for(AppType::Codex)
            .map_err(|source| AgentConfigurationError::DerivedArtifactPending { source })?;
        write_catalog(&path, &providers).map_err(|source| AgentConfigurationError::DerivedArtifactPending { source })?;
        transaction
            .commit()
            .map_err(|source| AgentConfigurationError::DerivedArtifactPending { source: source.into() })?;
        Ok(())
    }

    fn validate_codex_catalog(&self) -> anyhow::Result<()> {
        let providers = self.mgr.list_providers_for(AppType::Codex)?;
        build_catalog(&providers)?;
        Ok(())
    }

    fn catalog_transaction<T>(&self, action: &'static str, operation: impl FnOnce(&Transaction<'_>) -> anyhow::Result<T>) -> Result<T> {
        let transaction = Transaction::new_unchecked(self.mgr.db().conn(), TransactionBehavior::Immediate).map_err(|source| not_applied(action, source))?;
        let value = operation(&transaction).map_err(|source| not_applied(action, source))?;
        transaction.commit().map_err(|source| not_applied(action, source))?;
        Ok(value)
    }

    fn native_transaction<'b, T>(
        &self,
        action: &'static str,
        paths: impl IntoIterator<Item = &'b Path>,
        operation: impl FnOnce(&Transaction<'_>) -> anyhow::Result<T>,
    ) -> Result<T> {
        let transaction = Transaction::new_unchecked(self.mgr.db().conn(), TransactionBehavior::Immediate).map_err(|source| not_applied(action, source))?;
        let snapshot = NativeSnapshot::capture(paths).map_err(|source| not_applied(action, source))?;
        let value = match operation(&transaction) {
            Ok(value) => value,
            Err(source) => {
                drop(transaction);
                return Err(restore_after_failure(action, source, snapshot));
            }
        };
        if let Err(source) = transaction.commit() {
            return Err(restore_after_failure(action, source.into(), snapshot));
        }
        Ok(value)
    }
}

fn set_settings_in(transaction: &Transaction<'_>, settings: &[(&str, &str)]) -> anyhow::Result<()> {
    for (key, value) in settings {
        transaction.execute("INSERT OR REPLACE INTO settings (key, value) VALUES (?1, ?2)", params![key, value])?;
    }
    Ok(())
}

fn not_applied(action: &'static str, source: impl Into<anyhow::Error>) -> AgentConfigurationError {
    AgentConfigurationError::NotApplied { action, source: source.into() }
}

fn restore_after_failure(action: &'static str, source: anyhow::Error, snapshot: NativeSnapshot) -> AgentConfigurationError {
    match snapshot.restore() {
        Ok(()) => not_applied(action, source),
        Err(restore_error) => AgentConfigurationError::ExternalStateUncertain {
            action,
            source: source.context(format!("Restoring the previous native files also failed: {restore_error:#}")),
        },
    }
}

struct NativeSnapshot(Vec<(PathBuf, Option<Vec<u8>>)>);

impl NativeSnapshot {
    fn capture<'b>(paths: impl IntoIterator<Item = &'b Path>) -> anyhow::Result<Self> {
        paths
            .into_iter()
            .map(|path| {
                let content = match std::fs::read(path) {
                    Ok(content) => Some(content),
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
                    Err(error) => return Err(error).with_context(|| format!("Failed to snapshot {}", path.display())),
                };
                Ok((path.to_path_buf(), content))
            })
            .collect::<anyhow::Result<Vec<_>>>()
            .map(Self)
    }

    fn restore(self) -> anyhow::Result<()> {
        for (path, content) in self.0.into_iter().rev() {
            match content {
                Some(content) => native::write_private_file(&path, &content)?,
                None => match std::fs::remove_file(&path) {
                    Ok(()) => {}
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(error) => return Err(error).with_context(|| format!("Failed to remove {} while restoring native files", path.display())),
                },
            }
        }
        Ok(())
    }
}
