use ccswitch::core::config::ConfigManager;
use ccswitch::core::models::{CodexCatalog, CodexModel, Provider, Source, SwitchMode};
use ccswitch::core::switcher::{remove_codex_provider, switch_codex_model, switch_codex_provider, switch_profile};
use ccswitch::db::Db;
use std::fs;
use tempfile::tempdir;

#[test]
fn test_switch_local_writes_settings_json() {
    let dir = tempdir().unwrap();
    let defaults_path = dir.path().join("defaults.toml");
    fs::write(
        &defaults_path,
        r#"
version = 1
[[claude_providers]]
id = "p1"
name = "Test"
api_url = "https://api.test.com"
api_key = "sk-test-key"
[[claude_providers.profiles]]
id = "prof1"
name = "Default"
opus = "opus-model"
sonnet = "sonnet-model"
haiku = "haiku-model"
subagent = "subagent-model"
default = true
"#,
    )
    .unwrap();

    let db_path = dir.path().join("test.db");
    let settings_path = dir.path().join("settings.json");
    let mgr = ConfigManager::new(&db_path, Some(&defaults_path)).unwrap();
    let config = switch_profile(&mgr, "p1", "prof1", SwitchMode::Local, Some(&settings_path)).unwrap();

    assert_eq!(config.opus, "opus-model");
    assert_eq!(config.sonnet, "sonnet-model");
    assert_eq!(config.haiku, "haiku-model");
    assert_eq!(config.subagent, "subagent-model");
    assert_eq!(config.auth_token, "sk-test-key");

    let content = fs::read_to_string(&settings_path).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&content).unwrap();
    assert_eq!(parsed["env"]["ANTHROPIC_MODEL"], "sonnet-model");
    assert_eq!(parsed["env"]["ANTHROPIC_DEFAULT_OPUS_MODEL"], "opus-model");
    assert_eq!(parsed["env"]["ANTHROPIC_DEFAULT_SONNET_MODEL"], "sonnet-model");
    assert_eq!(parsed["env"]["ANTHROPIC_DEFAULT_HAIKU_MODEL"], "haiku-model");
    assert_eq!(parsed["env"]["CLAUDE_CODE_SUBAGENT_MODEL"], "subagent-model");
    assert_eq!(parsed["env"]["ANTHROPIC_BASE_URL"], "https://api.test.com");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(fs::metadata(&settings_path).unwrap().permissions().mode() & 0o777, 0o600);
    }
}

#[test]
fn custom_codex_model_writes_aggregated_catalog_and_model_config() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("ccswitch.db");
    let db = Db::open(&db_path).unwrap();
    let provider = Provider {
        id: "third-party".into(),
        name: "Third Party".into(),
        api_url: "https://api.example.com".into(),
        api_key: "sk-third-party".into(),
        codex_catalog: CodexCatalog::Custom,
        profiles: vec![],
        models: vec![],
        source: Source::User,
    };
    db.insert_provider(&provider, "codex").unwrap();
    let model = CodexModel {
        slug: "third-party-coder".into(),
        display_name: "Third-party Coder".into(),
        description: "Agentic coding model".into(),
        context_window: 128_000,
        max_context_window: Some(256_000),
        effective_context_window_percent: 95,
        default_reasoning_effort: "high".into(),
        supported_reasoning_efforts: vec!["low".into(), "high".into()],
        input_modalities: vec!["text".into()],
        supports_parallel_tool_calls: true,
        support_verbosity: true,
        default_verbosity: "low".into(),
        supports_search_tool: false,
        default: true,
        source: Source::User,
    };
    db.insert_codex_model(&provider.id, &model).unwrap();
    drop(db);
    let defaults_path = dir.path().join("missing-defaults.toml");
    let mgr = ConfigManager::new(&db_path, Some(&defaults_path)).unwrap();
    let config_path = dir.path().join("codex/config.toml");
    let auth_path = dir.path().join("codex/auth.json");
    switch_codex_model(&mgr, &provider.id, Some(&model.slug), Some(&config_path), Some(&auth_path)).unwrap();

    let config: toml::Value = toml::from_str(&fs::read_to_string(&config_path).unwrap()).unwrap();
    assert_eq!(config["model"].as_str(), Some("third-party-coder"));
    assert_eq!(config["model_reasoning_effort"].as_str(), Some("high"));
    let catalog_path = config["model_catalog_json"].as_str().unwrap();
    let catalog: serde_json::Value = serde_json::from_str(&fs::read_to_string(catalog_path).unwrap()).unwrap();
    assert_eq!(catalog["models"][0]["slug"], "third-party-coder");
    assert_eq!(catalog["models"][0]["context_window"], 128_000);
    assert_eq!(mgr.get_setting("active_codex_model").as_deref(), Some("third-party-coder"));
}

#[test]
fn switching_from_custom_to_builtin_removes_ccswitch_model_fields() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("ccswitch.db");
    let db = Db::open(&db_path).unwrap();
    for provider in [
        Provider {
            id: "custom".into(),
            name: "Custom".into(),
            api_url: "https://custom.example.com".into(),
            api_key: "sk-custom".into(),
            codex_catalog: CodexCatalog::Custom,
            profiles: vec![],
            models: vec![],
            source: Source::User,
        },
        Provider {
            id: "builtin".into(),
            name: "Builtin".into(),
            api_url: "https://api.openai.com/v1".into(),
            api_key: "sk-builtin".into(),
            codex_catalog: CodexCatalog::BuiltIn,
            profiles: vec![],
            models: vec![],
            source: Source::User,
        },
    ] {
        db.insert_provider(&provider, "codex").unwrap();
    }
    let model = CodexModel {
        slug: "custom-coder".into(),
        display_name: "Custom Coder".into(),
        description: String::new(),
        context_window: 128_000,
        max_context_window: None,
        effective_context_window_percent: 95,
        default_reasoning_effort: "medium".into(),
        supported_reasoning_efforts: vec!["medium".into()],
        input_modalities: vec!["text".into()],
        supports_parallel_tool_calls: true,
        support_verbosity: true,
        default_verbosity: "low".into(),
        supports_search_tool: false,
        default: true,
        source: Source::User,
    };
    db.insert_codex_model("custom", &model).unwrap();
    drop(db);
    let defaults_path = dir.path().join("missing-defaults.toml");
    let mgr = ConfigManager::new(&db_path, Some(&defaults_path)).unwrap();
    let config_path = dir.path().join("codex/config.toml");
    let auth_path = dir.path().join("codex/auth.json");
    switch_codex_model(&mgr, "custom", Some("custom-coder"), Some(&config_path), Some(&auth_path)).unwrap();
    switch_codex_model(&mgr, "builtin", None, Some(&config_path), Some(&auth_path)).unwrap();
    let config: toml::Value = toml::from_str(&fs::read_to_string(config_path).unwrap()).unwrap();
    assert!(config.get("model_catalog_json").is_none());
    assert!(config.get("model").is_none());
    assert!(config.get("model_reasoning_effort").is_none());
}

#[test]
fn invalid_claude_settings_are_not_overwritten() {
    let dir = tempdir().unwrap();
    let defaults_path = dir.path().join("defaults.toml");
    fs::write(
        &defaults_path,
        r#"
version = 1
[[claude_providers]]
id = "p1"
name = "Test"
api_url = "https://api.test.com"
api_key = "sk-test-key"
[[claude_providers.profiles]]
id = "prof1"
name = "Default"
opus = "opus"
sonnet = "sonnet"
haiku = "haiku"
subagent = "subagent"
"#,
    )
    .unwrap();
    let settings_path = dir.path().join("settings.json");
    fs::write(&settings_path, "{ invalid json").unwrap();
    let mgr = ConfigManager::new(&dir.path().join("ccswitch.db"), Some(&defaults_path)).unwrap();

    let error = switch_profile(&mgr, "p1", "prof1", SwitchMode::Local, Some(&settings_path)).unwrap_err();
    assert!(error.to_string().contains("Failed to parse Claude settings.json"));
    assert_eq!(fs::read_to_string(&settings_path).unwrap(), "{ invalid json");
}

#[test]
fn test_switch_proxy_updates_sqlite() {
    let dir = tempdir().unwrap();
    let defaults_path = dir.path().join("defaults.toml");
    fs::write(
        &defaults_path,
        r#"
version = 1
[[claude_providers]]
id = "p1"
name = "Test"
api_url = "https://api.test.com"
api_key = "env:TEST_KEY"
[[claude_providers.profiles]]
id = "prof1"
name = "Default"
opus = "opus-model"
sonnet = "sonnet-model"
haiku = "haiku-model"
subagent = "subagent-model"
"#,
    )
    .unwrap();

    std::env::set_var("TEST_KEY", "resolved-key");
    let db_path = dir.path().join("test.db");
    let settings_path = dir.path().join("settings.json");

    let mgr = ConfigManager::new(&db_path, Some(&defaults_path)).unwrap();
    let config = switch_profile(&mgr, "p1", "prof1", SwitchMode::Proxy, Some(&settings_path)).unwrap();

    assert_eq!(config.base_url, "http://127.0.0.1:15721");
    assert_eq!(mgr.get_setting("active_provider"), Some("p1".into()));
    assert_eq!(mgr.get_setting("active_profile"), Some("prof1".into()));
    assert_eq!(mgr.get_setting("proxy_mode"), Some("true".into()));

    let content = fs::read_to_string(&settings_path).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&content).unwrap();
    assert_eq!(parsed["env"]["CLAUDE_CODE_SUBAGENT_MODEL"], "ccswitch-subagent");
}

#[test]
fn test_switch_codex_provider_preserves_existing_config() {
    let dir = tempdir().unwrap();
    let defaults_path = dir.path().join("defaults.toml");
    fs::write(&defaults_path, "version = 1\n").unwrap();
    let db_path = dir.path().join("ccswitch.db");
    let db = Db::open(&db_path).unwrap();
    db.insert_provider(
        &Provider {
            id: "codex-proxy".into(),
            name: "Codex Proxy".into(),
            api_url: "https://codex.example.com/v1".into(),
            api_key: "sk-codex".into(),
            codex_catalog: Default::default(),
            profiles: vec![],
            models: vec![],
            source: Source::User,
        },
        "codex",
    )
    .unwrap();
    drop(db);
    let mgr = ConfigManager::new(&db_path, Some(&defaults_path)).unwrap();

    let config_path = dir.path().join("config.toml");
    let auth_path = dir.path().join("auth.json");
    fs::write(
        &config_path,
        r#"# keep this comment
model = "gpt-test"
[model_providers.existing]
name = "Existing"
base_url = "https://existing.example.com"

[model_providers.codex-proxy]
name = "Legacy Session Provider"
base_url = "https://legacy.example.com/v1"
wire_api = "responses"
requires_openai_auth = true

[model_providers.ccs]
name = "Old CCSwitch Provider"
base_url = "https://old.example.com/v1"
wire_api = "responses"
requires_openai_auth = true
"#,
    )
    .unwrap();
    fs::write(&auth_path, r#"{"other":"preserved"}"#).unwrap();

    switch_codex_provider(&mgr, "codex-proxy", Some(&config_path), Some(&auth_path)).unwrap();

    let config_text = fs::read_to_string(&config_path).unwrap();
    assert!(config_text.contains("# keep this comment"));
    assert!(!config_text.contains("\n[ccswitch]\n"));
    assert!(config_text.contains("[ccswitch.last_switch]"));
    let config: toml::Value = toml::from_str(&config_text).unwrap();
    assert_eq!(config["model"].as_str(), Some("gpt-test"));
    assert_eq!(config["model_provider"].as_str(), Some("ccs"));
    assert_eq!(config["model_providers"]["existing"]["name"].as_str(), Some("Existing"));
    assert_eq!(config["model_providers"]["ccs"]["name"].as_str(), Some("Codex Proxy"));
    assert_eq!(config["model_providers"]["ccs"]["base_url"].as_str(), Some("https://codex.example.com/v1"));
    assert_eq!(config["model_providers"]["ccs"]["wire_api"].as_str(), Some("responses"));
    assert_eq!(config["model_providers"]["ccs"]["requires_openai_auth"].as_bool(), Some(true));
    assert_eq!(config["model_providers"]["codex-proxy"]["base_url"].as_str(), Some("https://legacy.example.com/v1"));
    assert_eq!(config["ccswitch"]["last_switch"]["source"].as_str(), Some("codex-proxy"));

    let auth: serde_json::Value = serde_json::from_str(&fs::read_to_string(&auth_path).unwrap()).unwrap();
    assert_eq!(auth["OPENAI_API_KEY"], "sk-codex");
    assert_eq!(auth["other"], "preserved");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(fs::metadata(&auth_path).unwrap().permissions().mode() & 0o777, 0o600);
    }
    assert_eq!(mgr.get_setting("active_codex_provider"), Some("codex-proxy".into()));

    remove_codex_provider(&mgr, "codex-proxy", Some(&config_path)).unwrap();
    let removed_text = fs::read_to_string(&config_path).unwrap();
    let removed: toml::Value = toml::from_str(&removed_text).unwrap();
    assert_eq!(removed["model_provider"].as_str(), Some("ccs"));
    assert_eq!(removed["model_providers"]["ccs"]["base_url"].as_str(), Some("https://codex.example.com/v1"));
    assert_eq!(removed["model_providers"]["codex-proxy"]["base_url"].as_str(), Some("https://legacy.example.com/v1"));
    assert_eq!(removed["model_providers"]["existing"]["name"].as_str(), Some("Existing"));
    assert!(removed.get("ccswitch").and_then(|item| item.get("last_switch")).is_none());
    assert_eq!(mgr.get_setting("active_codex_provider"), Some(String::new()));

    let new_config_path = dir.path().join("new/config.toml");
    let new_auth_path = dir.path().join("new/auth.json");
    switch_codex_provider(&mgr, "codex-proxy", Some(&new_config_path), Some(&new_auth_path)).unwrap();
    assert!(new_config_path.exists());
    assert!(new_auth_path.exists());
    let new_config: toml::Value = toml::from_str(&fs::read_to_string(&new_config_path).unwrap()).unwrap();
    assert_eq!(new_config["model_provider"].as_str(), Some("ccs"));
    assert_eq!(new_config["model_providers"]["ccs"]["base_url"].as_str(), Some("https://codex.example.com/v1"));

    let guarded_config_path = dir.path().join("guarded-config.toml");
    let corrupt_auth_path = dir.path().join("corrupt-auth.json");
    let original_config = "model = \"preserved\"\n";
    fs::write(&guarded_config_path, original_config).unwrap();
    fs::write(&corrupt_auth_path, "{ invalid json").unwrap();
    assert!(switch_codex_provider(&mgr, "codex-proxy", Some(&guarded_config_path), Some(&corrupt_auth_path),).is_err());
    assert_eq!(fs::read_to_string(&guarded_config_path).unwrap(), original_config);

    let invalid_table_path = dir.path().join("invalid-table.toml");
    let invalid_table = "model_providers = \"do not overwrite\"\n";
    fs::write(&invalid_table_path, invalid_table).unwrap();
    assert!(switch_codex_provider(&mgr, "codex-proxy", Some(&invalid_table_path), Some(&new_auth_path),).is_err());
    assert_eq!(fs::read_to_string(&invalid_table_path).unwrap(), invalid_table);
}
