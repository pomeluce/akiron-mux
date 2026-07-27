use ccswitch::core::models::{validate_profile, validate_provider, Profile, Provider, Source};

#[test]
fn test_provider_deserialization() {
    let toml_str = r#"
id = "deepseek"
name = "DeepSeek"
api_url = "https://api.deepseek.com/anthropic"
api_key = "env:DEEPSEEK_API_KEY"
"#;
    let p: Provider = toml::from_str(toml_str).unwrap();
    assert_eq!(p.id, "deepseek");
    assert_eq!(p.name, "DeepSeek");
    assert_eq!(p.api_url, "https://api.deepseek.com/anthropic");
    assert_eq!(p.api_key, "env:DEEPSEEK_API_KEY");
}

#[test]
fn test_profile_deserialization() {
    let toml_str = r#"
id = "v4"
name = "V4"
opus = "deepseek-v4-pro[1m]"
sonnet = "deepseek-v4-pro[1m]"
haiku = "deepseek-v4-flash"
subagent = "deepseek-v4-flash"
default = true
"#;
    let p: Profile = toml::from_str(toml_str).unwrap();
    assert_eq!(p.id, "v4");
    assert_eq!(p.opus, "deepseek-v4-pro[1m]");
    assert_eq!(p.sonnet, "deepseek-v4-pro[1m]");
    assert_eq!(p.haiku, "deepseek-v4-flash");
    assert_eq!(p.subagent, "deepseek-v4-flash");
    assert!(p.default);
}

#[test]
fn test_source_system_cannot_delete() {
    let s = Source::System;
    assert!(!s.can_delete());
}

#[test]
fn test_source_user_can_delete() {
    let s = Source::User;
    assert!(s.can_delete());
}

#[test]
fn provider_validation_rejects_unsafe_ids_urls_and_env_refs() {
    let provider = |id: &str, api_url: &str, api_key: &str| Provider {
        id: id.into(),
        name: "Provider".into(),
        api_url: api_url.into(),
        api_key: api_key.into(),
        profiles: vec![],
        source: Source::User,
    };

    assert!(validate_provider(&provider(
        "safe-id",
        "https://api.example.com/v1",
        "env:API_KEY"
    ))
    .is_ok());
    assert!(validate_provider(&provider("bad id", "https://api.example.com", "key")).is_err());
    assert!(validate_provider(&provider("safe", "file:///tmp/socket", "key")).is_err());
    assert!(validate_provider(&provider("safe", "https://user:pass@example.com", "key")).is_err());
    assert!(validate_provider(&provider(
        "safe",
        "https://api.example.com?token=secret",
        "key"
    ))
    .is_err());
    assert!(
        validate_provider(&provider("safe", "https://api.example.com", "env:BAD-NAME")).is_err()
    );
}

#[test]
fn profile_validation_requires_all_four_models() {
    let mut profile = Profile {
        id: "default".into(),
        name: "Default".into(),
        opus: "opus".into(),
        sonnet: "sonnet".into(),
        haiku: "haiku".into(),
        subagent: "subagent".into(),
        default: false,
        source: Source::User,
    };
    assert!(validate_profile(&profile).is_ok());
    profile.subagent.clear();
    assert!(validate_profile(&profile).is_err());
}
