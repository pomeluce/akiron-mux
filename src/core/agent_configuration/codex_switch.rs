use super::native::write_private_file;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Serialize, Deserialize)]
pub(super) struct CodexSwitch {
    pub source: String,
    pub model: Option<String>,
    pub at: String,
    provider_name: Option<String>,
    base_url: Option<String>,
}

pub(super) fn state_path(config_path: &Path) -> PathBuf {
    config_path.parent().unwrap_or_else(|| Path::new(".")).join("akmux/last-switch.json")
}

impl CodexSwitch {
    pub fn new(config: &toml_edit::DocumentMut, source: String, model: Option<String>, at: String) -> Self {
        let provider = config.get("model_provider").and_then(toml_edit::Item::as_str).unwrap_or("openai");
        let table = config.get("model_providers").and_then(|providers| providers.get(provider));
        Self {
            source,
            model,
            at,
            provider_name: table.and_then(|table| table.get("name")).and_then(toml_edit::Item::as_str).map(str::to_owned),
            base_url: table.and_then(|table| table.get("base_url")).and_then(toml_edit::Item::as_str).map(str::to_owned),
        }
    }

    pub fn matches_config(&self, config: &toml_edit::DocumentMut) -> bool {
        let provider = config.get("model_provider").and_then(toml_edit::Item::as_str).unwrap_or("openai");
        if !matches!(provider, "akmux" | "ccs" | "openai") {
            return false;
        }
        let table = config.get("model_providers").and_then(|providers| providers.get(provider));
        self.base_url.is_some()
            && self.base_url.as_deref() == table.and_then(|table| table.get("base_url")).and_then(toml_edit::Item::as_str)
            && self.provider_name.as_deref() == table.and_then(|table| table.get("name")).and_then(toml_edit::Item::as_str)
    }

    pub fn write(&self, config_path: &Path) -> Result<()> {
        write_private_file(&state_path(config_path), &serde_json::to_vec_pretty(self)?)
    }
}

fn legacy_switch(config: &toml_edit::DocumentMut) -> Option<CodexSwitch> {
    ["akmux", "ccswitch"].into_iter().find_map(|namespace| {
        let switch = config.get(namespace)?.get("last_switch")?;
        Some(CodexSwitch::new(
            config,
            switch.get("source")?.as_str()?.to_owned(),
            switch.get("model").and_then(toml_edit::Item::as_str).map(str::to_owned),
            switch.get("at").and_then(toml_edit::Item::as_str).unwrap_or("").to_owned(),
        ))
    })
}

pub(super) fn read(config_path: &Path, config: &toml_edit::DocumentMut) -> Result<Option<CodexSwitch>> {
    if let Some(state) = legacy_switch(config) {
        return Ok(Some(state));
    }
    let path = state_path(config_path);
    match std::fs::read(&path) {
        Ok(content) => serde_json::from_slice(&content).context("Failed to parse Codex switch metadata").map(Some),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error).with_context(|| format!("Failed to read {}", path.display())),
    }
}

pub(super) fn remove_legacy(config: &mut toml_edit::DocumentMut) {
    for namespace in ["akmux", "ccswitch"] {
        let remove_namespace = config.get_mut(namespace).and_then(toml_edit::Item::as_table_mut).is_some_and(|table| {
            table.remove("last_switch");
            table.is_empty()
        });
        if remove_namespace {
            config.remove(namespace);
        }
    }
}

pub(super) fn migrate(config_path: &Path) -> Result<()> {
    let content = match std::fs::read_to_string(config_path) {
        Ok(content) => content,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error).with_context(|| format!("Failed to read {}", config_path.display())),
    };
    let mut config: toml_edit::DocumentMut = content.parse().context("Failed to parse Codex config.toml during migration")?;
    if let Some(state) = legacy_switch(&config) {
        state.write(config_path)?;
        remove_legacy(&mut config);
        write_private_file(config_path, config.to_string().as_bytes())?;
    }
    Ok(())
}
