use ccswitch::core::agent_configuration::{AgentConfigPaths, AgentConfiguration};
use ccswitch::core::config::ConfigManager;
use ccswitch::core::models::{AppType, CodexCatalog, CodexModel, Provider, Source, SwitchMode, OFFICIAL_CODEX_PROVIDER_ID};
use ccswitch::db::Db;
use std::fs;
use tempfile::tempdir;

fn claude_configuration<'a>(mgr: &'a ConfigManager, settings_path: &std::path::Path) -> AgentConfiguration<'a> {
    AgentConfiguration::with_paths(
        mgr,
        AgentConfigPaths::new(
            settings_path.to_path_buf(),
            settings_path.with_file_name("unused-config.toml"),
            settings_path.with_file_name("unused-auth.json"),
        ),
    )
}

fn codex_configuration<'a>(mgr: &'a ConfigManager, config_path: &std::path::Path, auth_path: &std::path::Path) -> AgentConfiguration<'a> {
    AgentConfiguration::with_paths(
        mgr,
        AgentConfigPaths::new(config_path.with_file_name("unused-settings.json"), config_path.to_path_buf(), auth_path.to_path_buf()),
    )
}

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
    fs::write(
        &settings_path,
        r#"{
  "ccswitch": {
    "last_switch": { "source": "legacy/profile" }
  },
  "last_switch": { "source": "older/profile" }
}"#,
    )
    .unwrap();
    let mgr = ConfigManager::new(&db_path, Some(&defaults_path)).unwrap();
    let config = claude_configuration(&mgr, &settings_path).apply_claude_profile("p1", "prof1", SwitchMode::Local).unwrap();

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
    assert_eq!(parsed["akmux"]["last_switch"]["source"], "p1/prof1");
    assert!(parsed.get("ccswitch").is_none());
    assert!(parsed.get("last_switch").is_none());
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
    codex_configuration(&mgr, &config_path, &auth_path)
        .apply_codex_model(&provider.id, Some(&model.slug))
        .unwrap();

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
fn switching_from_custom_to_builtin_removes_managed_model_fields() {
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
    let configuration = codex_configuration(&mgr, &config_path, &auth_path);
    configuration.apply_codex_model("custom", Some("custom-coder")).unwrap();
    configuration.apply_codex_model("builtin", None).unwrap();
    let config: toml::Value = toml::from_str(&fs::read_to_string(config_path).unwrap()).unwrap();
    assert!(config.get("model_catalog_json").is_none());
    assert!(config.get("model").is_none());
    assert!(config.get("model_reasoning_effort").is_none());
}

#[test]
fn switching_between_official_and_third_party_codex_preserves_separate_auth_files() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("akmux.db");
    let db = Db::open(&db_path).unwrap();
    db.insert_provider(
        &Provider {
            id: "third-party".into(),
            name: "Third Party".into(),
            api_url: "https://api.example.com/v1".into(),
            api_key: "sk-third-party".into(),
            codex_catalog: CodexCatalog::BuiltIn,
            profiles: vec![],
            models: vec![],
            source: Source::User,
        },
        "codex",
    )
    .unwrap();
    drop(db);
    let mgr = ConfigManager::new(&db_path, Some(&dir.path().join("missing-defaults.toml"))).unwrap();
    let codex_dir = dir.path().join("codex");
    fs::create_dir_all(&codex_dir).unwrap();
    let config_path = codex_dir.join("config.toml");
    let auth_path = codex_dir.join("auth.json");
    let official_auth = r#"{"auth_mode":"chatgpt","tokens":{"access_token":"official-session"}}"#;
    fs::write(&config_path, "model_provider = \"openai\"\nmodel = \"gpt-5\"\n").unwrap();
    fs::write(&auth_path, official_auth).unwrap();

    let configuration = codex_configuration(&mgr, &config_path, &auth_path);
    configuration.apply_codex_provider("third-party").unwrap();

    let third_party_config: toml::Value = toml::from_str(&fs::read_to_string(&config_path).unwrap()).unwrap();
    assert_eq!(third_party_config["model_provider"].as_str(), Some("akmux"));
    assert_eq!(third_party_config["model_providers"]["akmux"]["base_url"].as_str(), Some("https://api.example.com/v1"));
    assert_eq!(fs::read_to_string(codex_dir.join("auth_openai.json")).unwrap(), official_auth);
    let third_party_auth: serde_json::Value = serde_json::from_str(&fs::read_to_string(&auth_path).unwrap()).unwrap();
    assert_eq!(third_party_auth["OPENAI_API_KEY"], "sk-third-party");

    configuration.apply_codex_provider(OFFICIAL_CODEX_PROVIDER_ID).unwrap();

    let official_config: toml::Value = toml::from_str(&fs::read_to_string(&config_path).unwrap()).unwrap();
    assert_eq!(official_config["model_provider"].as_str(), Some("akmux"));
    assert_eq!(official_config["model_providers"]["akmux"]["name"].as_str(), Some("OpenAI"));
    assert_eq!(
        official_config["model_providers"]["akmux"]["base_url"].as_str(),
        Some("https://chatgpt.com/backend-api/codex")
    );
    assert_eq!(official_config["akmux"]["last_switch"]["source"].as_str(), Some(OFFICIAL_CODEX_PROVIDER_ID));
    assert_eq!(fs::read_to_string(&auth_path).unwrap(), official_auth);
    let akmux_auth: serde_json::Value = serde_json::from_str(&fs::read_to_string(codex_dir.join("auth_akmux.json")).unwrap()).unwrap();
    assert_eq!(akmux_auth["OPENAI_API_KEY"], "sk-third-party");
    assert!(!codex_dir.join("auth_openai.json").exists());
    assert_eq!(mgr.get_setting("active_codex_provider").as_deref(), Some(OFFICIAL_CODEX_PROVIDER_ID));
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

    let error = claude_configuration(&mgr, &settings_path)
        .apply_claude_profile("p1", "prof1", SwitchMode::Local)
        .unwrap_err();
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
    let config = claude_configuration(&mgr, &settings_path).apply_claude_profile("p1", "prof1", SwitchMode::Proxy).unwrap();

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

[model_providers.openai]
name = "Old AkironMux Provider"
base_url = "https://old-openai.example.com/v1"
wire_api = "responses"
requires_openai_auth = true

[ccswitch.last_switch]
source = "legacy-provider"
"#,
    )
    .unwrap();
    fs::write(&auth_path, r#"{"other":"preserved"}"#).unwrap();

    let configuration = codex_configuration(&mgr, &config_path, &auth_path);
    configuration.apply_codex_provider("codex-proxy").unwrap();

    let config_text = fs::read_to_string(&config_path).unwrap();
    assert!(config_text.contains("# keep this comment"));
    assert!(!config_text.contains("\n[akmux]\n"));
    assert!(config_text.contains("[akmux.last_switch]"));
    assert!(!config_text.contains("[ccswitch.last_switch]"));
    let config: toml::Value = toml::from_str(&config_text).unwrap();
    assert_eq!(config["model"].as_str(), Some("gpt-test"));
    assert_eq!(config["model_provider"].as_str(), Some("akmux"));
    assert_eq!(config["model_providers"]["existing"]["name"].as_str(), Some("Existing"));
    assert_eq!(config["model_providers"]["akmux"]["name"].as_str(), Some("Codex Proxy"));
    assert_eq!(config["model_providers"]["akmux"]["base_url"].as_str(), Some("https://codex.example.com/v1"));
    assert_eq!(config["model_providers"]["akmux"]["wire_api"].as_str(), Some("responses"));
    assert_eq!(config["model_providers"]["akmux"]["requires_openai_auth"].as_bool(), Some(true));
    assert!(config["model_providers"].get("openai").is_none());
    assert!(config["model_providers"].get("ccs").is_none());
    assert_eq!(config["model_providers"]["codex-proxy"]["base_url"].as_str(), Some("https://legacy.example.com/v1"));
    assert_eq!(config["akmux"]["last_switch"]["source"].as_str(), Some("codex-proxy"));

    let auth: serde_json::Value = serde_json::from_str(&fs::read_to_string(&auth_path).unwrap()).unwrap();
    assert_eq!(auth["OPENAI_API_KEY"], "sk-codex");
    assert!(auth.get("other").is_none());
    assert_eq!(fs::read_to_string(dir.path().join("auth_openai.json")).unwrap(), r#"{"other":"preserved"}"#);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(fs::metadata(&auth_path).unwrap().permissions().mode() & 0o777, 0o600);
    }
    assert_eq!(mgr.get_setting("active_codex_provider"), Some("codex-proxy".into()));

    let error = configuration.delete_provider(AppType::Codex, "codex-proxy").unwrap_err();
    assert!(error.to_string().contains("Cannot delete active provider"));
    configuration.apply_codex_provider(OFFICIAL_CODEX_PROVIDER_ID).unwrap();
    configuration.delete_provider(AppType::Codex, "codex-proxy").unwrap();
    assert!(mgr.find_provider_for(AppType::Codex, "codex-proxy").unwrap().is_none());

    let new_config_path = dir.path().join("new/config.toml");
    let new_auth_path = dir.path().join("new/auth.json");
    configuration
        .save_provider(
            AppType::Codex,
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
        )
        .unwrap();
    codex_configuration(&mgr, &new_config_path, &new_auth_path).apply_codex_provider("codex-proxy").unwrap();
    assert!(new_config_path.exists());
    assert!(new_auth_path.exists());
    let new_config: toml::Value = toml::from_str(&fs::read_to_string(&new_config_path).unwrap()).unwrap();
    assert_eq!(new_config["model_provider"].as_str(), Some("akmux"));
    assert_eq!(new_config["model_providers"]["akmux"]["base_url"].as_str(), Some("https://codex.example.com/v1"));

    let guarded_dir = dir.path().join("guarded");
    fs::create_dir_all(&guarded_dir).unwrap();
    let guarded_config_path = guarded_dir.join("config.toml");
    let corrupt_auth_path = guarded_dir.join("auth.json");
    let original_config = "model = \"preserved\"\n";
    fs::write(&guarded_config_path, original_config).unwrap();
    fs::write(&corrupt_auth_path, "{ invalid json").unwrap();
    codex_configuration(&mgr, &guarded_config_path, &corrupt_auth_path)
        .apply_codex_provider("codex-proxy")
        .unwrap();
    assert_eq!(fs::read_to_string(guarded_dir.join("auth_openai.json")).unwrap(), "{ invalid json");
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&fs::read_to_string(&corrupt_auth_path).unwrap()).unwrap()["OPENAI_API_KEY"],
        "sk-codex"
    );

    let invalid_table_path = dir.path().join("invalid-table.toml");
    let invalid_table = "model_providers = \"do not overwrite\"\n";
    fs::write(&invalid_table_path, invalid_table).unwrap();
    assert!(codex_configuration(&mgr, &invalid_table_path, &new_auth_path).apply_codex_provider("codex-proxy").is_err());
    assert_eq!(fs::read_to_string(&invalid_table_path).unwrap(), invalid_table);
}

fn user_claude_provider() -> Provider {
    Provider {
        id: "user-provider".into(),
        name: "User Provider".into(),
        api_url: "https://api.example.com".into(),
        api_key: "sk-user".into(),
        codex_catalog: Default::default(),
        profiles: vec![],
        models: vec![],
        source: Source::User,
    }
}

fn user_claude_profile(sonnet: &str) -> ccswitch::core::models::Profile {
    ccswitch::core::models::Profile {
        id: "user-profile".into(),
        name: "User Profile".into(),
        opus: "opus-model".into(),
        sonnet: sonnet.into(),
        haiku: "haiku-model".into(),
        subagent: "subagent-model".into(),
        default: false,
        source: Source::User,
    }
}

#[test]
fn active_profile_cannot_be_deleted() {
    let directory = tempdir().unwrap();
    let mgr = ConfigManager::new(&directory.path().join("akmux.db"), Some(&directory.path().join("missing-defaults.toml"))).unwrap();
    let settings_path = directory.path().join("claude/settings.json");
    let configuration = claude_configuration(&mgr, &settings_path);
    configuration.save_provider(AppType::Claude, &user_claude_provider()).unwrap();
    configuration.save_profile("user-provider", &user_claude_profile("sonnet-before")).unwrap();
    configuration.apply_claude_profile("user-provider", "user-profile", SwitchMode::Local).unwrap();

    let error = configuration.delete_profile("user-provider", "user-profile").unwrap_err();

    assert!(error.to_string().contains("Cannot delete active model profile"));
    assert!(mgr.find_profile("user-provider", "user-profile").unwrap().is_some());
}

#[test]
fn editing_active_profile_reapplies_native_configuration() {
    let directory = tempdir().unwrap();
    let mgr = ConfigManager::new(&directory.path().join("akmux.db"), Some(&directory.path().join("missing-defaults.toml"))).unwrap();
    let settings_path = directory.path().join("claude/settings.json");
    let configuration = claude_configuration(&mgr, &settings_path);
    configuration.save_provider(AppType::Claude, &user_claude_provider()).unwrap();
    configuration.save_profile("user-provider", &user_claude_profile("sonnet-before")).unwrap();
    configuration.apply_claude_profile("user-provider", "user-profile", SwitchMode::Local).unwrap();

    configuration.save_profile("user-provider", &user_claude_profile("sonnet-after")).unwrap();

    let settings: serde_json::Value = serde_json::from_str(&fs::read_to_string(settings_path).unwrap()).unwrap();
    assert_eq!(settings["env"]["ANTHROPIC_MODEL"], "sonnet-after");
    assert_eq!(mgr.find_profile("user-provider", "user-profile").unwrap().unwrap().1.sonnet, "sonnet-after");
}

#[test]
fn failed_active_profile_write_rolls_back_catalog_change() {
    let directory = tempdir().unwrap();
    let mgr = ConfigManager::new(&directory.path().join("akmux.db"), Some(&directory.path().join("missing-defaults.toml"))).unwrap();
    let settings_path = directory.path().join("claude/settings.json");
    let configuration = claude_configuration(&mgr, &settings_path);
    configuration.save_provider(AppType::Claude, &user_claude_provider()).unwrap();
    configuration.save_profile("user-provider", &user_claude_profile("sonnet-before")).unwrap();
    configuration.apply_claude_profile("user-provider", "user-profile", SwitchMode::Local).unwrap();
    fs::write(&settings_path, "{ invalid json").unwrap();

    let error = configuration.save_profile("user-provider", &user_claude_profile("sonnet-after")).unwrap_err();

    assert!(error.to_string().contains("Failed to parse Claude settings.json"));
    assert_eq!(mgr.find_profile("user-provider", "user-profile").unwrap().unwrap().1.sonnet, "sonnet-before");
    assert_eq!(fs::read_to_string(settings_path).unwrap(), "{ invalid json");
}

#[test]
fn reconcile_rebuilds_active_projection_from_native_configuration() {
    let directory = tempdir().unwrap();
    let mgr = ConfigManager::new(&directory.path().join("akmux.db"), Some(&directory.path().join("missing-defaults.toml"))).unwrap();
    let settings_path = directory.path().join("claude/settings.json");
    let configuration = claude_configuration(&mgr, &settings_path);
    configuration.save_provider(AppType::Claude, &user_claude_provider()).unwrap();
    configuration.save_profile("user-provider", &user_claude_profile("sonnet-model")).unwrap();
    configuration.apply_claude_profile("user-provider", "user-profile", SwitchMode::Proxy).unwrap();
    mgr.set_setting("active_provider", "stale-provider").unwrap();
    mgr.set_setting("active_profile", "stale-profile").unwrap();
    mgr.set_setting("proxy_mode", "false").unwrap();

    configuration.reconcile().unwrap();

    assert_eq!(mgr.get_setting("active_provider").as_deref(), Some("user-provider"));
    assert_eq!(mgr.get_setting("active_profile").as_deref(), Some("user-profile"));
    assert_eq!(mgr.get_setting("proxy_mode").as_deref(), Some("true"));
}

#[test]
fn reconcile_keeps_agents_independent_when_one_native_file_is_invalid() {
    let directory = tempdir().unwrap();
    let mgr = ConfigManager::new(&directory.path().join("akmux.db"), Some(&directory.path().join("missing-defaults.toml"))).unwrap();
    let settings_path = directory.path().join("settings.json");
    fs::write(&settings_path, "{ invalid json").unwrap();
    mgr.set_setting("active_codex_provider", "stale-provider").unwrap();
    let configuration = claude_configuration(&mgr, &settings_path);

    let error = configuration.reconcile().unwrap_err();

    assert!(error.to_string().contains("Claude"));
    assert_eq!(mgr.get_setting("active_codex_provider").as_deref(), Some(OFFICIAL_CODEX_PROVIDER_ID));
}
