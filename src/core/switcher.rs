use crate::core::config::ConfigManager;
use crate::core::env::{resolve_api_key, resolve_codex_api_key, ApiKeyUnavailable};
use crate::core::models::{
    validate_profile, validate_provider, ActiveConfig, AppType, Provider, SwitchMode,
};
use anyhow::{Context, Result};
use serde_json::json;
use std::io::Write;
use std::path::Path;

const DEFAULT_PROXY_PORT: u16 = 15721;

pub fn switch_profile(
    mgr: &ConfigManager,
    provider_id: &str,
    profile_id: &str,
    mode: SwitchMode,
    settings_path: Option<&Path>,
) -> Result<ActiveConfig> {
    let (provider, profile) = mgr
        .find_profile(provider_id, profile_id)?
        .with_context(|| format!("Profile not found: {}/{}", provider_id, profile_id))?;
    validate_provider(&provider)?;
    validate_profile(&profile)?;

    let auth_token = resolve_api_key(&provider.api_key);
    if auth_token.is_empty() {
        return Err(
            ApiKeyUnavailable::new(&provider.id, &provider.api_key, "CLAUDE_API_KEY").into(),
        );
    }
    let base_url = match mode {
        SwitchMode::Proxy => format!("http://127.0.0.1:{}", DEFAULT_PROXY_PORT),
        SwitchMode::Local => provider.api_url.clone(),
    };

    tracing::info!(
        "switch_profile: mode={:?} provider={} profile={} base_url={}",
        mode,
        provider_id,
        profile_id,
        base_url
    );

    let config = ActiveConfig {
        provider_id: provider.id.clone(),
        profile_id: profile.id.clone(),
        provider_name: provider.name.clone(),
        profile_name: profile.name.clone(),
        base_url: base_url.clone(),
        auth_token: auth_token.clone(),
        opus: profile.opus.clone(),
        sonnet: profile.sonnet.clone(),
        haiku: profile.haiku.clone(),
        subagent: profile.subagent.clone(),
    };

    write_settings_json(&config, mode, settings_path)?;
    mgr.set_setting("active_provider", &config.provider_id)?;
    mgr.set_setting("active_profile", &config.profile_id)?;
    mgr.set_setting(
        "proxy_mode",
        if mode == SwitchMode::Proxy {
            "true"
        } else {
            "false"
        },
    )?;
    if mode == SwitchMode::Proxy {
        mgr.set_setting("proxy_port", &DEFAULT_PROXY_PORT.to_string())?;
    }
    Ok(config)
}

fn write_settings_json(config: &ActiveConfig, mode: SwitchMode, path: Option<&Path>) -> Result<()> {
    let settings_path = path.map(|p| p.to_path_buf()).unwrap_or_else(|| {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
        Path::new(&home).join(".claude").join("settings.json")
    });

    tracing::debug!(
        "write_settings_json: path={} base_url={} auth_token={} reasoning={} task={}",
        settings_path.display(),
        config.base_url,
        if config.auth_token.is_empty() {
            "(empty)"
        } else {
            "(set)"
        },
        config.opus,
        config.haiku,
    );

    let mut existing: serde_json::Value = if settings_path.exists() {
        let content = std::fs::read_to_string(&settings_path)?;
        serde_json::from_str(&content).context("Failed to parse Claude settings.json")?
    } else {
        json!({})
    };

    if let Some(parent) = settings_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    if !existing.is_object() {
        anyhow::bail!("Claude settings.json root must be an object");
    }
    if existing["env"].is_null() || !existing["env"].is_object() {
        existing["env"] = json!({});
    }
    let env = &mut existing["env"];

    // Always write the base URL (proxy address or upstream URL)
    env["ANTHROPIC_BASE_URL"] = json!(config.base_url);

    match mode {
        SwitchMode::Local => {
            // Local: write model vars + auth token for Claude Code
            env["ANTHROPIC_AUTH_TOKEN"] = json!(config.auth_token);
            env["ANTHROPIC_MODEL"] = json!(config.sonnet);
            env["ANTHROPIC_DEFAULT_OPUS_MODEL"] = json!(config.opus);
            env["ANTHROPIC_DEFAULT_SONNET_MODEL"] = json!(config.sonnet);
            env["ANTHROPIC_DEFAULT_HAIKU_MODEL"] = json!(config.haiku.replace("[1m]", ""));
            env["CLAUDE_CODE_SUBAGENT_MODEL"] = json!(config.subagent.replace("[1m]", ""));
        }
        SwitchMode::Proxy => {
            // Proxy: set dummy auth token (Claude Code requires it to skip login),
            // remove model vars — proxy server handles model routing
            env["ANTHROPIC_AUTH_TOKEN"] = json!("ccswitch-proxy");
            let model_keys = [
                "ANTHROPIC_MODEL",
                "ANTHROPIC_DEFAULT_OPUS_MODEL",
                "ANTHROPIC_DEFAULT_SONNET_MODEL",
                "ANTHROPIC_DEFAULT_HAIKU_MODEL",
            ];
            for k in &model_keys {
                env.as_object_mut().and_then(|o| o.remove(*k));
            }
            // Use a stable marker so the proxy can distinguish subagent calls
            // from ordinary Opus/Sonnet/Haiku requests.
            env["CLAUDE_CODE_SUBAGENT_MODEL"] = json!("ccswitch-subagent");
        }
    }

    let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
    existing["last_switch"] = json!({
        "source": format!("{}/{}", config.provider_id, config.profile_id),
        "mode": match mode { SwitchMode::Local => "local", SwitchMode::Proxy => "proxy" },
        "at": now,
    });

    write_private_file(
        &settings_path,
        serde_json::to_string_pretty(&existing)?.as_bytes(),
    )?;
    tracing::debug!("write_settings_json: wrote to {}", settings_path.display());
    Ok(())
}

/// Switch Codex to a provider and update ~/.codex/config.toml + auth.json.
/// Existing unrelated settings and provider definitions are preserved.
pub fn switch_codex_provider(
    mgr: &ConfigManager,
    provider_id: &str,
    config_path: Option<&Path>,
    auth_path: Option<&Path>,
) -> Result<Provider> {
    let provider = mgr
        .find_provider_for(AppType::Codex, provider_id)?
        .with_context(|| format!("Codex provider not found: {}", provider_id))?;
    validate_provider(&provider)?;
    let auth_token = resolve_codex_api_key(&provider.api_key);
    if auth_token.is_empty() {
        return Err(
            ApiKeyUnavailable::new(&provider.id, &provider.api_key, "OPENAI_API_KEY").into(),
        );
    }

    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    let codex_dir = Path::new(&home).join(".codex");
    let config_path = config_path
        .map(Path::to_path_buf)
        .unwrap_or_else(|| codex_dir.join("config.toml"));
    let auth_path = auth_path
        .map(Path::to_path_buf)
        .unwrap_or_else(|| codex_dir.join("auth.json"));

    if let Some(parent) = config_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    if let Some(parent) = auth_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let mut config: toml_edit::DocumentMut = if config_path.exists() {
        let content = std::fs::read_to_string(&config_path)?;
        content
            .parse()
            .context("Failed to parse Codex config.toml")?
    } else {
        toml_edit::DocumentMut::new()
    };
    let mut auth: serde_json::Value = if auth_path.exists() {
        let content = std::fs::read_to_string(&auth_path)?;
        serde_json::from_str(&content).context("Failed to parse Codex auth.json")?
    } else {
        json!({})
    };
    if !auth.is_object() {
        anyhow::bail!("Codex auth.json root must be an object");
    }

    config["model_provider"] = toml_edit::value(&provider.id);
    if config
        .as_table()
        .get("model_providers")
        .is_some_and(|item| !item.is_table())
    {
        anyhow::bail!("Codex config.toml 'model_providers' must be a table");
    }
    if config.as_table().get("model_providers").is_none() {
        config.as_table_mut().insert(
            "model_providers",
            toml_edit::Item::Table(toml_edit::Table::new()),
        );
    }
    let providers = config["model_providers"]
        .as_table_mut()
        .expect("table created above");
    if providers.iter().all(|(_, item)| item.is_table()) {
        providers.set_implicit(true);
    }
    if providers
        .get(&provider.id)
        .is_some_and(|item| !item.is_table())
    {
        anyhow::bail!("Codex provider '{}' must be a table", provider.id);
    }
    if providers.get(&provider.id).is_none() {
        providers.insert(
            &provider.id,
            toml_edit::Item::Table(toml_edit::Table::new()),
        );
    }
    let provider_table = providers
        .get_mut(&provider.id)
        .and_then(toml_edit::Item::as_table_mut)
        .expect("provider table created above");
    provider_table.insert("name", toml_edit::value(&provider.name));
    provider_table.insert("base_url", toml_edit::value(&provider.api_url));
    provider_table.insert("wire_api", toml_edit::value("responses"));
    provider_table.insert("requires_openai_auth", toml_edit::value(true));

    let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
    if config
        .as_table()
        .get("ccswitch")
        .is_some_and(|item| !item.is_table())
    {
        anyhow::bail!("Codex config.toml 'ccswitch' must be a table");
    }
    if config.as_table().get("ccswitch").is_none() {
        config
            .as_table_mut()
            .insert("ccswitch", toml_edit::Item::Table(toml_edit::Table::new()));
    }
    let ccswitch = config["ccswitch"]
        .as_table_mut()
        .expect("table created above");
    if ccswitch
        .get("last_switch")
        .is_some_and(|item| !item.is_table())
    {
        anyhow::bail!("Codex config.toml 'ccswitch.last_switch' must be a table");
    }
    if ccswitch.get("last_switch").is_none() {
        ccswitch.insert(
            "last_switch",
            toml_edit::Item::Table(toml_edit::Table::new()),
        );
    }
    let last_switch = ccswitch
        .get_mut("last_switch")
        .and_then(toml_edit::Item::as_table_mut)
        .expect("last_switch table created above");
    last_switch.insert("source", toml_edit::value(&provider.id));
    last_switch.insert("at", toml_edit::value(now));
    if ccswitch
        .iter()
        .all(|(key, item)| key == "last_switch" && item.is_table())
    {
        ccswitch.set_implicit(true);
    }
    std::fs::write(&config_path, config.to_string())?;

    auth["OPENAI_API_KEY"] = json!(auth_token);
    write_private_file(&auth_path, serde_json::to_string_pretty(&auth)?.as_bytes())?;

    mgr.set_setting(AppType::Codex.active_provider_key(), &provider.id)?;
    Ok(provider)
}

/// Remove a Codex provider definition from config.toml while preserving all
/// unrelated settings, comments, and other providers.
pub fn remove_codex_provider(
    mgr: &ConfigManager,
    provider_id: &str,
    config_path: Option<&Path>,
) -> Result<()> {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    let config_path = config_path
        .map(Path::to_path_buf)
        .unwrap_or_else(|| Path::new(&home).join(".codex/config.toml"));
    if config_path.exists() {
        let content = std::fs::read_to_string(&config_path)?;
        let mut config: toml_edit::DocumentMut = content
            .parse()
            .context("Failed to parse Codex config.toml")?;
        if config
            .as_table()
            .get("model_providers")
            .is_some_and(|item| !item.is_table())
        {
            anyhow::bail!("Codex config.toml 'model_providers' must be a table");
        }
        if let Some(providers) = config
            .as_table_mut()
            .get_mut("model_providers")
            .and_then(toml_edit::Item::as_table_mut)
        {
            providers.remove(provider_id);
        }
        let is_selected = config
            .as_table()
            .get("model_provider")
            .and_then(toml_edit::Item::as_str)
            == Some(provider_id);
        if is_selected {
            config.as_table_mut().remove("model_provider");
        }
        let last_source_matches = config
            .as_table()
            .get("ccswitch")
            .and_then(toml_edit::Item::as_table)
            .and_then(|table| table.get("last_switch"))
            .and_then(toml_edit::Item::as_table)
            .and_then(|table| table.get("source"))
            .and_then(toml_edit::Item::as_str)
            == Some(provider_id);
        if last_source_matches {
            if let Some(ccswitch) = config
                .as_table_mut()
                .get_mut("ccswitch")
                .and_then(toml_edit::Item::as_table_mut)
            {
                ccswitch.remove("last_switch");
            }
        }
        std::fs::write(config_path, config.to_string())?;
    }
    if mgr
        .get_setting(AppType::Codex.active_provider_key())
        .as_deref()
        == Some(provider_id)
    {
        mgr.set_setting(AppType::Codex.active_provider_key(), "")?;
    }
    Ok(())
}

fn write_private_file(path: &Path, content: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut options = std::fs::OpenOptions::new();
    options.create(true).truncate(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    }
    file.write_all(content)?;
    file.sync_all()?;
    Ok(())
}
