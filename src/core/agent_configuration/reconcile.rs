use crate::core::config::ConfigManager;
use crate::core::models::{AppType, OFFICIAL_CODEX_PROVIDER_ID};
use anyhow::{Context, Result};
use std::path::Path;

pub(super) fn reconcile_claude(mgr: &ConfigManager, settings_path: &Path) -> Result<()> {
    let content = match std::fs::read_to_string(settings_path) {
        Ok(content) => content,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error).with_context(|| format!("Failed to read {}", settings_path.display())),
    };
    let parsed: serde_json::Value = serde_json::from_str(&content).context("Failed to parse Claude settings.json during reconciliation")?;
    let managed_switch = parsed
        .get("akmux")
        .and_then(|value| value.get("last_switch"))
        .or_else(|| parsed.get("ccswitch").and_then(|value| value.get("last_switch")))
        .or_else(|| parsed.get("last_switch"));
    let source = managed_switch.and_then(|value| value.get("source")).and_then(serde_json::Value::as_str).unwrap_or("");
    let Some((provider_id, profile_id)) = source.split_once('/') else {
        return Ok(());
    };
    if mgr.find_profile(provider_id, profile_id)?.is_none() {
        return Ok(());
    }
    let proxy_mode = (managed_switch.and_then(|value| value.get("mode")).and_then(serde_json::Value::as_str) == Some("proxy")).to_string();
    mgr.set_settings(&[("active_provider", provider_id), ("active_profile", profile_id), ("proxy_mode", proxy_mode.as_str())])?;
    Ok(())
}

pub(super) fn reconcile_codex(mgr: &ConfigManager, config_path: &Path) -> Result<()> {
    let content = match std::fs::read_to_string(config_path) {
        Ok(content) => content,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return select_official_codex(mgr),
        Err(error) => return Err(error).with_context(|| format!("Failed to read {}", config_path.display())),
    };
    let parsed: toml::Value = toml::from_str(&content).context("Failed to parse Codex config.toml during reconciliation")?;
    let managed_switch = parsed
        .get("akmux")
        .and_then(|value| value.get("last_switch"))
        .or_else(|| parsed.get("ccswitch").and_then(|value| value.get("last_switch")))
        .and_then(toml::Value::as_table);
    let provider_id = managed_switch
        .and_then(|switch| switch.get("source"))
        .and_then(toml::Value::as_str)
        .or_else(|| parsed.get("model_provider").and_then(toml::Value::as_str));
    let Some(provider_id) = provider_id else {
        return select_official_codex(mgr);
    };
    let model = managed_switch.and_then(|switch| switch.get("model")).and_then(toml::Value::as_str).unwrap_or_else(|| {
        if managed_switch.is_some() {
            ""
        } else {
            parsed.get("model").and_then(toml::Value::as_str).unwrap_or("")
        }
    });
    if mgr.find_provider_for(AppType::Codex, provider_id)?.is_some() {
        mgr.set_settings(&[(AppType::Codex.active_provider_key(), provider_id), ("active_codex_model", model)])?;
    }
    Ok(())
}

fn select_official_codex(mgr: &ConfigManager) -> Result<()> {
    mgr.set_settings(&[(AppType::Codex.active_provider_key(), OFFICIAL_CODEX_PROVIDER_ID), ("active_codex_model", "")])?;
    Ok(())
}
