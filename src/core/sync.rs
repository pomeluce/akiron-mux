use crate::core::config::ConfigManager;
use crate::core::models::AppType;

/// Sync active provider/profile from Claude Code's settings.json (last_switch.source).
/// Called on app startup to align CCSwitch's active selection with the last switch.
pub fn sync_active_from_settings(mgr: &ConfigManager) {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    let settings_path = std::path::PathBuf::from(&home).join(".claude/settings.json");
    if !settings_path.exists() {
        return;
    }
    let content = match std::fs::read_to_string(&settings_path) {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!("Failed to read settings.json for sync: {}", e);
            return;
        }
    };
    let parsed: serde_json::Value = match serde_json::from_str(&content) {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!("Failed to parse settings.json for sync: {}", e);
            return;
        }
    };
    let source = parsed.get("last_switch").and_then(|v| v.get("source")).and_then(|v| v.as_str()).unwrap_or("");
    if let Some((pid, pfid)) = source.split_once('/') {
        let providers = match mgr.list_providers() {
            Ok(p) => p,
            Err(e) => {
                tracing::warn!("Failed to list providers for sync: {}", e);
                return;
            }
        };
        if providers.iter().any(|p| p.id == pid && p.profiles.iter().any(|pr| pr.id == pfid)) {
            if let Err(e) = mgr.set_setting("active_provider", pid) {
                tracing::warn!("sync: failed to save active_provider: {}", e);
            }
            if let Err(e) = mgr.set_setting("active_profile", pfid) {
                tracing::warn!("sync: failed to save active_profile: {}", e);
            }
        }

        // Restore proxy_mode from last switch
        let mode = parsed.get("last_switch").and_then(|v| v.get("mode")).and_then(|v| v.as_str()).unwrap_or("local");
        let is_proxy = mode == "proxy";
        if let Err(e) = mgr.set_setting("proxy_mode", &is_proxy.to_string()) {
            tracing::warn!("sync: failed to save proxy_mode: {}", e);
        }
    }
}

/// Sync the active Codex provider from ~/.codex/config.toml.
pub fn sync_codex_active_from_config(mgr: &ConfigManager) {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    let config_path = std::path::PathBuf::from(&home).join(".codex/config.toml");
    sync_codex_active_from_path(mgr, &config_path);
}

fn sync_codex_active_from_path(mgr: &ConfigManager, config_path: &std::path::Path) {
    let content = match std::fs::read_to_string(config_path) {
        Ok(content) => content,
        Err(_) => return,
    };
    let parsed: toml::Value = match toml::from_str(&content) {
        Ok(value) => value,
        Err(e) => {
            tracing::warn!("Failed to parse Codex config.toml for sync: {}", e);
            return;
        }
    };
    let managed_switch = parsed.get("ccswitch").and_then(|value| value.get("last_switch")).and_then(toml::Value::as_table);
    let provider_id = managed_switch
        .and_then(|switch| switch.get("source"))
        .and_then(toml::Value::as_str)
        .or_else(|| parsed.get("model_provider").and_then(toml::Value::as_str));
    let Some(provider_id) = provider_id else {
        return;
    };
    let model = if let Some(switch) = managed_switch {
        switch.get("model").and_then(toml::Value::as_str).unwrap_or("")
    } else {
        parsed.get("model").and_then(toml::Value::as_str).unwrap_or("")
    };

    match mgr.find_provider_for(AppType::Codex, provider_id) {
        Ok(Some(_)) => {
            if let Err(e) = mgr.set_setting(AppType::Codex.active_provider_key(), provider_id) {
                tracing::warn!("Failed to sync active Codex provider: {}", e);
            }
            if let Err(e) = mgr.set_setting("active_codex_model", model) {
                tracing::warn!("Failed to sync active Codex model: {}", e);
            }
        }
        Ok(None) => {}
        Err(e) => tracing::warn!("Failed to list Codex providers for sync: {}", e),
    }
}

#[cfg(test)]
mod tests {
    use super::sync_codex_active_from_path;
    use crate::core::config::ConfigManager;
    use crate::core::models::{AppType, Provider, Source};
    use tempfile::tempdir;

    #[test]
    fn managed_builtin_switch_does_not_import_unrelated_model() {
        let dir = tempdir().unwrap();
        let defaults_path = dir.path().join("missing-defaults.toml");
        let mgr = ConfigManager::new(&dir.path().join("ccswitch.db"), Some(&defaults_path)).unwrap();
        mgr.db()
            .insert_provider(
                &Provider {
                    id: "builtin".into(),
                    name: "Builtin".into(),
                    api_url: "https://api.example.com".into(),
                    api_key: "key".into(),
                    codex_catalog: Default::default(),
                    profiles: vec![],
                    models: vec![],
                    source: Source::User,
                },
                AppType::Codex.as_str(),
            )
            .unwrap();
        let config_path = dir.path().join("config.toml");
        std::fs::write(
            &config_path,
            r#"model_provider = "ccs"
model = "user-model"

[ccswitch.last_switch]
source = "builtin"
"#,
        )
        .unwrap();

        sync_codex_active_from_path(&mgr, &config_path);
        assert_eq!(mgr.get_setting("active_codex_provider").as_deref(), Some("builtin"));
        assert_eq!(mgr.get_setting("active_codex_model").as_deref(), Some(""));
    }
}
