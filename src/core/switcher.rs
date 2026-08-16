use crate::core::codex_catalog::{default_catalog_path, write_catalog};
use crate::core::config::ConfigManager;
use crate::core::env::{resolve_api_key, resolve_codex_api_key, ApiKeyUnavailable};
use crate::core::models::{validate_profile, validate_provider, ActiveConfig, AppType, Provider, SwitchMode, OFFICIAL_CODEX_BASE_URL, OFFICIAL_CODEX_PROVIDER_ID};
use anyhow::{Context, Result};
use serde_json::json;
use std::io::Write;
use std::path::Path;

const DEFAULT_PROXY_PORT: u16 = 15721;
const CODEX_MANAGED_PROVIDER_ID: &str = "akmux";
const LEGACY_CODEX_MANAGED_PROVIDER_IDS: [&str; 2] = ["openai", "ccs"];
const OFFICIAL_CODEX_AUTH_BACKUP: &str = "auth_openai.json";
const AKMUX_CODEX_AUTH_BACKUP: &str = "auth_akmux.json";

pub fn switch_profile(mgr: &ConfigManager, provider_id: &str, profile_id: &str, mode: SwitchMode, settings_path: Option<&Path>) -> Result<ActiveConfig> {
    let (provider, profile) = mgr
        .find_profile(provider_id, profile_id)?
        .with_context(|| format!("Profile not found: {}/{}", provider_id, profile_id))?;
    validate_provider(&provider)?;
    validate_profile(&profile)?;

    let auth_token = resolve_api_key(&provider.api_key);
    if auth_token.is_empty() {
        return Err(ApiKeyUnavailable::new(&provider.id, &provider.api_key, "CLAUDE_API_KEY").into());
    }
    let base_url = match mode {
        SwitchMode::Proxy => format!("http://127.0.0.1:{}", DEFAULT_PROXY_PORT),
        SwitchMode::Local => provider.api_url.clone(),
    };

    tracing::info!("switch_profile: mode={:?} provider={} profile={} base_url={}", mode, provider_id, profile_id, base_url);

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
    mgr.set_setting("proxy_mode", if mode == SwitchMode::Proxy { "true" } else { "false" })?;
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
        if config.auth_token.is_empty() { "(empty)" } else { "(set)" },
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
    if !existing.get("akmux").map_or(true, serde_json::Value::is_object) {
        anyhow::bail!("Claude settings.json 'akmux' must be an object");
    }
    if existing.get("akmux").is_none() {
        existing["akmux"] = json!({});
    }
    existing["akmux"]["last_switch"] = json!({
        "source": format!("{}/{}", config.provider_id, config.profile_id),
        "mode": match mode { SwitchMode::Local => "local", SwitchMode::Proxy => "proxy" },
        "at": now,
    });
    existing.as_object_mut().expect("root object validated above").remove("last_switch");
    if let Some(legacy) = existing.get_mut("ccswitch").and_then(serde_json::Value::as_object_mut) {
        legacy.remove("last_switch");
        if legacy.is_empty() {
            existing.as_object_mut().expect("root object validated above").remove("ccswitch");
        }
    }

    write_private_file(&settings_path, serde_json::to_string_pretty(&existing)?.as_bytes())?;
    tracing::debug!("write_settings_json: wrote to {}", settings_path.display());
    Ok(())
}

/// Switch Codex to a provider and update ~/.codex/config.toml + auth.json.
/// Official and third-party sessions share the reserved `akmux` provider ID;
/// authentication files are swapped independently from the provider metadata.
#[allow(dead_code)]
pub fn switch_codex_provider(mgr: &ConfigManager, provider_id: &str, config_path: Option<&Path>, auth_path: Option<&Path>) -> Result<Provider> {
    let provider = mgr
        .find_provider_for(AppType::Codex, provider_id)?
        .with_context(|| format!("Codex provider not found: {}", provider_id))?;
    validate_provider(&provider)?;
    let model_slug = provider
        .models
        .iter()
        .find(|model| model.default)
        .or_else(|| provider.models.first())
        .map(|model| model.slug.as_str());
    switch_codex_model(mgr, provider_id, model_slug, config_path, auth_path)
}

pub fn switch_codex_model(mgr: &ConfigManager, provider_id: &str, model_slug: Option<&str>, config_path: Option<&Path>, auth_path: Option<&Path>) -> Result<Provider> {
    let provider = mgr
        .find_provider_for(AppType::Codex, provider_id)?
        .with_context(|| format!("Codex provider not found: {}", provider_id))?;
    validate_provider(&provider)?;
    let official = provider.id == OFFICIAL_CODEX_PROVIDER_ID;
    let selected_model = if !official && provider.codex_catalog == crate::core::models::CodexCatalog::Custom {
        let slug = model_slug.context("This third-party Codex provider has no configured model")?;
        Some(
            provider
                .models
                .iter()
                .find(|model| model.slug == slug)
                .with_context(|| format!("Codex model not found: {}/{}", provider_id, slug))?,
        )
    } else {
        None
    };
    let auth_token = if official {
        None
    } else {
        let token = resolve_codex_api_key(&provider.api_key);
        if token.is_empty() {
            return Err(ApiKeyUnavailable::new(&provider.id, &provider.api_key, "OPENAI_API_KEY").into());
        }
        Some(token)
    };

    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    let codex_dir = Path::new(&home).join(".codex");
    let config_path = config_path.map(Path::to_path_buf).unwrap_or_else(|| codex_dir.join("config.toml"));
    let auth_path = auth_path.map(Path::to_path_buf).unwrap_or_else(|| codex_dir.join("auth.json"));
    let catalog_path = config_path.parent().map(|dir| dir.join("akmux/models.json")).unwrap_or_else(default_catalog_path);

    if let Some(parent) = config_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    if let Some(parent) = auth_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let mut config: toml_edit::DocumentMut = if config_path.exists() {
        let content = std::fs::read_to_string(&config_path)?;
        content.parse().context("Failed to parse Codex config.toml")?
    } else {
        toml_edit::DocumentMut::new()
    };
    let previously_third_party = uses_third_party_codex_provider(&config);
    let previous_managed_model = managed_last_switch(&config)
        .and_then(|table| table.get("model"))
        .and_then(toml_edit::Item::as_str)
        .map(str::to_owned);
    if let Some(model) = selected_model {
        let all_providers = mgr.list_providers_for(AppType::Codex)?;
        write_catalog(&catalog_path, &all_providers)?;
        config["model"] = toml_edit::value(&model.slug);
        config["model_reasoning_effort"] = toml_edit::value(&model.default_reasoning_effort);
        config["model_catalog_json"] = toml_edit::value(catalog_path.to_string_lossy().to_string());
    } else if config
        .get("model_catalog_json")
        .and_then(toml_edit::Item::as_str)
        .is_some_and(|path| path == catalog_path.to_string_lossy())
    {
        config.as_table_mut().remove("model_catalog_json");
    }
    if selected_model.is_none() && previous_managed_model.as_deref() == config.get("model").and_then(toml_edit::Item::as_str) {
        config.as_table_mut().remove("model");
        config.as_table_mut().remove("model_reasoning_effort");
    }

    configure_managed_codex_provider(&mut config, &provider, selected_model)?;
    if official {
        restore_official_codex_auth(&auth_path, previously_third_party)?;
    } else {
        activate_akmux_codex_auth(&auth_path, auth_token.as_deref().expect("non-official provider resolved a token"), previously_third_party)?;
    }
    write_private_file(&config_path, config.to_string().as_bytes())?;

    mgr.set_setting(AppType::Codex.active_provider_key(), &provider.id)?;
    mgr.set_setting("active_codex_model", selected_model.map(|model| model.slug.as_str()).unwrap_or(""))?;
    Ok(provider)
}

fn configure_managed_codex_provider(config: &mut toml_edit::DocumentMut, provider: &Provider, selected_model: Option<&crate::core::models::CodexModel>) -> Result<()> {
    config["model_provider"] = toml_edit::value(CODEX_MANAGED_PROVIDER_ID);
    if config.as_table().get("model_providers").is_some_and(|item| !item.is_table()) {
        anyhow::bail!("Codex config.toml 'model_providers' must be a table");
    }
    if config.as_table().get("model_providers").is_none() {
        config.as_table_mut().insert("model_providers", toml_edit::Item::Table(toml_edit::Table::new()));
    }
    let providers = config["model_providers"].as_table_mut().expect("table created above");
    if providers.iter().all(|(_, item)| item.is_table()) {
        providers.set_implicit(true);
    }
    for legacy_id in LEGACY_CODEX_MANAGED_PROVIDER_IDS {
        providers.remove(legacy_id);
    }
    if providers.get(CODEX_MANAGED_PROVIDER_ID).is_some_and(|item| !item.is_table()) {
        anyhow::bail!("Codex provider '{}' must be a table", CODEX_MANAGED_PROVIDER_ID);
    }
    if providers.get(CODEX_MANAGED_PROVIDER_ID).is_none() {
        providers.insert(CODEX_MANAGED_PROVIDER_ID, toml_edit::Item::Table(toml_edit::Table::new()));
    }
    let provider_table = providers
        .get_mut(CODEX_MANAGED_PROVIDER_ID)
        .and_then(toml_edit::Item::as_table_mut)
        .expect("provider table created above");
    provider_table.insert("name", toml_edit::value(&provider.name));
    provider_table.insert("base_url", toml_edit::value(&provider.api_url));
    provider_table.insert("wire_api", toml_edit::value("responses"));
    provider_table.insert("requires_openai_auth", toml_edit::value(true));

    if config.as_table().get("akmux").is_some_and(|item| !item.is_table()) {
        anyhow::bail!("Codex config.toml 'akmux' must be a table");
    }
    if config.as_table().get("akmux").is_none() {
        config.as_table_mut().insert("akmux", toml_edit::Item::Table(toml_edit::Table::new()));
    }
    let akmux = config["akmux"].as_table_mut().expect("table created above");
    if akmux.get("last_switch").is_some_and(|item| !item.is_table()) {
        anyhow::bail!("Codex config.toml 'akmux.last_switch' must be a table");
    }
    if akmux.get("last_switch").is_none() {
        akmux.insert("last_switch", toml_edit::Item::Table(toml_edit::Table::new()));
    }
    let last_switch = akmux
        .get_mut("last_switch")
        .and_then(toml_edit::Item::as_table_mut)
        .expect("last_switch table created above");
    last_switch.insert("source", toml_edit::value(&provider.id));
    if let Some(model) = selected_model {
        last_switch.insert("model", toml_edit::value(&model.slug));
    } else {
        last_switch.remove("model");
    }
    last_switch.insert("at", toml_edit::value(chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string()));
    if akmux.iter().all(|(key, item)| key == "last_switch" && item.is_table()) {
        akmux.set_implicit(true);
    }
    clear_legacy_last_switch(config);
    Ok(())
}

fn uses_third_party_codex_provider(config: &toml_edit::DocumentMut) -> bool {
    let provider = config.get("model_provider").and_then(toml_edit::Item::as_str);
    if !provider.is_some_and(is_managed_codex_provider_id) {
        return false;
    }
    let source = managed_last_switch(config).and_then(|table| table.get("source")).and_then(toml_edit::Item::as_str);
    if source == Some(OFFICIAL_CODEX_PROVIDER_ID) {
        return false;
    }
    if source.is_some() {
        return true;
    }
    let Some(provider_table) = config
        .get("model_providers")
        .and_then(toml_edit::Item::as_table)
        .and_then(|providers| provider.and_then(|provider| providers.get(provider)))
        .and_then(toml_edit::Item::as_table)
    else {
        return false;
    };
    provider_table.get("name").and_then(toml_edit::Item::as_str) != Some("OpenAI")
        || provider_table.get("base_url").and_then(toml_edit::Item::as_str) != Some(OFFICIAL_CODEX_BASE_URL)
}

fn is_managed_codex_provider_id(provider: &str) -> bool {
    provider == CODEX_MANAGED_PROVIDER_ID || LEGACY_CODEX_MANAGED_PROVIDER_IDS.contains(&provider)
}

fn clear_legacy_last_switch(config: &mut toml_edit::DocumentMut) {
    let remove_legacy = config.as_table_mut().get_mut("ccswitch").and_then(toml_edit::Item::as_table_mut).is_some_and(|legacy| {
        legacy.remove("last_switch");
        legacy.is_empty()
    });
    if remove_legacy {
        config.as_table_mut().remove("ccswitch");
    }
}

fn activate_akmux_codex_auth(auth_path: &Path, token: &str, previously_third_party: bool) -> Result<()> {
    if !previously_third_party && auth_path.exists() {
        move_private_file(auth_path, &auth_path.with_file_name(OFFICIAL_CODEX_AUTH_BACKUP))?;
    }
    write_private_file(auth_path, serde_json::to_string_pretty(&json!({ "OPENAI_API_KEY": token }))?.as_bytes())
}

fn restore_official_codex_auth(auth_path: &Path, previously_third_party: bool) -> Result<()> {
    if previously_third_party && auth_path.exists() {
        move_private_file(auth_path, &auth_path.with_file_name(AKMUX_CODEX_AUTH_BACKUP))?;
    }
    let official_backup = auth_path.with_file_name(OFFICIAL_CODEX_AUTH_BACKUP);
    if !auth_path.exists() && official_backup.exists() {
        move_private_file(&official_backup, auth_path)?;
    }
    Ok(())
}

/// Remove AkironMux's association with a Codex provider. Provider definitions
/// remain in config.toml because existing Codex sessions refer to them by ID.
pub fn remove_codex_provider(mgr: &ConfigManager, provider_id: &str, config_path: Option<&Path>) -> Result<()> {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    let config_path = config_path.map(Path::to_path_buf).unwrap_or_else(|| Path::new(&home).join(".codex/config.toml"));
    if config_path.exists() {
        let content = std::fs::read_to_string(&config_path)?;
        let mut config: toml_edit::DocumentMut = content.parse().context("Failed to parse Codex config.toml")?;
        let last_source_matches = managed_last_switch(&config).and_then(|table| table.get("source")).and_then(toml_edit::Item::as_str) == Some(provider_id);
        if last_source_matches {
            for namespace in ["akmux", "ccswitch"] {
                if let Some(table) = config.as_table_mut().get_mut(namespace).and_then(toml_edit::Item::as_table_mut) {
                    table.remove("last_switch");
                }
            }
        }
        write_private_file(&config_path, config.to_string().as_bytes())?;
    }
    if mgr.get_setting(AppType::Codex.active_provider_key()).as_deref() == Some(provider_id) {
        mgr.set_setting(AppType::Codex.active_provider_key(), "")?;
    }
    Ok(())
}

fn managed_last_switch(config: &toml_edit::DocumentMut) -> Option<&toml_edit::Table> {
    ["akmux", "ccswitch"].into_iter().find_map(|namespace| {
        config
            .as_table()
            .get(namespace)
            .and_then(toml_edit::Item::as_table)
            .and_then(|table| table.get("last_switch"))
            .and_then(toml_edit::Item::as_table)
    })
}

fn move_private_file(source: &Path, destination: &Path) -> Result<()> {
    let content = std::fs::read(source).with_context(|| format!("Failed to read {}", source.display()))?;
    write_private_file(destination, &content)?;
    std::fs::remove_file(source).with_context(|| format!("Failed to remove {} after creating its backup", source.display()))?;
    Ok(())
}

fn write_private_file(path: &Path, content: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let file_name = path.file_name().and_then(|name| name.to_str()).unwrap_or("config");
    let temporary = path.with_file_name(format!(".{}.{}.akmux.tmp", file_name, std::process::id()));
    let mut options = std::fs::OpenOptions::new();
    options.create(true).truncate(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&temporary)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    }
    file.write_all(content)?;
    file.sync_all()?;
    drop(file);
    #[cfg(windows)]
    if path.exists() {
        std::fs::remove_file(path)?;
    }
    std::fs::rename(temporary, path)?;
    Ok(())
}
