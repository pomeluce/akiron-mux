use ccswitch::core::config::ConfigManager;
use ccswitch::core::models::AppType;
use ccswitch::db::Db;
use std::fs;
use tempfile::tempdir;

#[test]
fn test_system_defaults_loaded() {
    let dir = tempdir().unwrap();
    let defaults_path = dir.path().join("defaults.toml");
    fs::write(
        &defaults_path,
        r#"
version = 1
[[providers]]
id = "test-provider"
name = "Test Provider"
api_url = "https://test.example.com"
api_key = "env:TEST_KEY"
[[providers.profiles]]
id = "test-profile"
name = "Test Profile"
opus = "model-opus"
sonnet = "model-sonnet"
haiku = "model-haiku"
subagent = "model-subagent"
default = true
"#,
    )
    .unwrap();

    let mgr = ConfigManager::new(&dir.path().join("ccswitch.db"), Some(&defaults_path)).unwrap();
    let providers = mgr.list_providers().unwrap();
    assert_eq!(providers.len(), 1);
    assert_eq!(providers[0].name, "Test Provider");
    assert_eq!(providers[0].source, ccswitch::core::models::Source::System);
    assert_eq!(providers[0].profiles.len(), 1);
    assert_eq!(providers[0].profiles[0].name, "Test Profile");
}

#[test]
fn test_codex_defaults_load_without_profiles() {
    let dir = tempdir().unwrap();
    let defaults_path = dir.path().join("defaults.toml");
    fs::write(
        &defaults_path,
        r#"
version = 1
[[codex_providers]]
id = "codex-proxy"
name = "Codex Proxy"
api_url = "https://codex.example.com/v1"
api_key = "env:OPENAI_API_KEY"
"#,
    )
    .unwrap();

    let mgr = ConfigManager::new(&dir.path().join("ccswitch.db"), Some(&defaults_path)).unwrap();
    let providers = mgr.list_providers_for(AppType::Codex).unwrap();
    assert_eq!(providers.len(), 1);
    assert_eq!(providers[0].id, "codex-proxy");
    assert!(providers[0].profiles.is_empty());
}

#[test]
fn test_custom_codex_models_load_from_defaults() {
    let dir = tempdir().unwrap();
    let defaults_path = dir.path().join("defaults.toml");
    fs::write(
        &defaults_path,
        r#"
version = 1
[[codex_providers]]
id = "third-party"
name = "Third Party"
api_url = "https://api.example.com"
api_key = "env:THIRD_PARTY_KEY"
codex_catalog = "custom"

[[codex_providers.models]]
slug = "third-party-coder"
display_name = "Third-party Coder"
context_window = 128000
default_reasoning_effort = "high"
supported_reasoning_efforts = ["low", "high"]
default = true
"#,
    )
    .unwrap();
    let mgr = ConfigManager::new(&dir.path().join("ccswitch.db"), Some(&defaults_path)).unwrap();
    let providers = mgr.list_providers_for(AppType::Codex).unwrap();
    assert_eq!(providers[0].codex_catalog, ccswitch::core::models::CodexCatalog::Custom);
    assert_eq!(providers[0].models.len(), 1);
    assert_eq!(providers[0].models[0].slug, "third-party-coder");
}

#[test]
fn removed_system_codex_models_do_not_remain_in_database() {
    let dir = tempdir().unwrap();
    let defaults_path = dir.path().join("defaults.toml");
    fs::write(
        &defaults_path,
        r#"
[[codex_providers]]
id = "third-party"
name = "Third Party"
api_url = "https://api.example.com"
api_key = "env:THIRD_PARTY_KEY"
codex_catalog = "custom"
[[codex_providers.models]]
slug = "old-model"
display_name = "Old Model"
"#,
    )
    .unwrap();
    let db_path = dir.path().join("ccswitch.db");
    ConfigManager::new(&db_path, Some(&defaults_path)).unwrap();
    fs::write(
        &defaults_path,
        r#"
[[codex_providers]]
id = "third-party"
name = "Third Party"
api_url = "https://api.example.com"
api_key = "env:THIRD_PARTY_KEY"
codex_catalog = "custom"
"#,
    )
    .unwrap();
    let mgr = ConfigManager::new(&db_path, Some(&defaults_path)).unwrap();
    assert!(mgr.list_providers_for(AppType::Codex).unwrap()[0].models.is_empty());
}

#[test]
fn test_user_override() {
    let dir = tempdir().unwrap();
    let defaults_path = dir.path().join("defaults.toml");
    fs::write(
        &defaults_path,
        r#"
version = 1
[[providers]]
id = "p1"
name = "System Provider"
api_url = "https://system.example.com"
api_key = "env:SYS_KEY"
[[providers.profiles]]
id = "prof1"
name = "System Profile"
opus = "sys-opus"
sonnet = "sys-sonnet"
haiku = "sys-haiku"
subagent = "sys-subagent"
"#,
    )
    .unwrap();

    // ConfigManager uses model.db (not test.db) — open and pre-populate that.
    let db = Db::open(&dir.path().join("ccswitch.db")).unwrap();
    // User adds a new profile under the system provider
    use ccswitch::core::models::{Provider, Source};
    db.insert_provider(
        &Provider {
            id: "p1".into(),
            name: "My Override".into(),
            api_url: "https://my.example.com".into(),
            api_key: "sk-xyz".into(),
            codex_catalog: Default::default(),
            profiles: vec![],
            models: vec![],
            source: Source::User,
        },
        "claude",
    )
    .unwrap();
    drop(db);

    let mgr = ConfigManager::new(&dir.path().join("ccswitch.db"), Some(&defaults_path)).unwrap();
    let providers = mgr.list_providers().unwrap();
    let p1 = providers.iter().find(|p| p.id == "p1").unwrap();
    // User override wins for provider fields
    assert_eq!(p1.name, "My Override");
    assert_eq!(p1.api_url, "https://my.example.com");
    // System profiles still present
    assert_eq!(p1.profiles.len(), 1);
    assert_eq!(p1.profiles[0].name, "System Profile");
}

#[test]
fn invalid_defaults_are_rejected_before_sync() {
    let dir = tempdir().unwrap();
    let defaults_path = dir.path().join("defaults.toml");
    fs::write(
        &defaults_path,
        r#"
version = 1
[[codex_providers]]
id = "invalid id"
name = "Invalid"
api_url = "file:///tmp/socket"
api_key = "env:BAD-NAME"
"#,
    )
    .unwrap();

    let error = ConfigManager::new(&dir.path().join("ccswitch.db"), Some(&defaults_path))
        .err()
        .expect("invalid defaults should fail");
    assert!(error.to_string().contains("Invalid provider"));
}
