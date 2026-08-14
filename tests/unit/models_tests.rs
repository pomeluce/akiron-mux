use ccswitch::core::models::{validate_codex_model, validate_codex_provider_models, validate_profile, validate_provider, CodexModel, Profile, Provider, Source};

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
fn codex_model_validation_checks_context_and_reasoning() {
    let mut model = CodexModel {
        slug: "third-party-coder".into(),
        display_name: "Third-party Coder".into(),
        description: String::new(),
        context_window: 128_000,
        max_context_window: Some(256_000),
        effective_context_window_percent: 95,
        default_reasoning_effort: "medium".into(),
        supported_reasoning_efforts: vec!["low".into(), "medium".into(), "high".into()],
        input_modalities: vec!["text".into()],
        supports_parallel_tool_calls: true,
        support_verbosity: true,
        default_verbosity: "low".into(),
        supports_search_tool: false,
        default: true,
        source: Source::User,
    };
    assert!(validate_codex_model(&model).is_ok());
    model.default_reasoning_effort = "max".into();
    assert!(validate_codex_model(&model).is_err());
    model.default_reasoning_effort = "medium".into();
    model.max_context_window = Some(64_000);
    assert!(validate_codex_model(&model).is_err());
    model.max_context_window = Some(256_000);
    model.supported_reasoning_efforts.push("medium".into());
    assert!(validate_codex_model(&model).is_err());
    model.supported_reasoning_efforts.pop();
    model.context_window = u64::MAX;
    model.max_context_window = Some(u64::MAX);
    assert!(validate_codex_model(&model).is_err());
}

#[test]
fn codex_provider_models_reject_duplicate_slugs_and_defaults() {
    let model = CodexModel {
        slug: "coder".into(),
        display_name: "Coder".into(),
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
    let mut provider = Provider {
        id: "provider".into(),
        name: "Provider".into(),
        api_url: "https://api.example.com".into(),
        api_key: "key".into(),
        codex_catalog: Default::default(),
        profiles: vec![],
        models: vec![model.clone(), model],
        source: Source::User,
    };
    assert!(validate_codex_provider_models(&provider).is_err());
    provider.models[1].slug = "coder-2".into();
    assert!(validate_codex_provider_models(&provider).is_err());
    provider.models[1].default = false;
    assert!(validate_codex_provider_models(&provider).is_ok());
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
        codex_catalog: Default::default(),
        profiles: vec![],
        models: vec![],
        source: Source::User,
    };

    assert!(validate_provider(&provider("safe-id", "https://api.example.com/v1", "env:API_KEY")).is_ok());
    assert!(validate_provider(&provider("bad id", "https://api.example.com", "key")).is_err());
    assert!(validate_provider(&provider("safe", "file:///tmp/socket", "key")).is_err());
    assert!(validate_provider(&provider("safe", "https://user:pass@example.com", "key")).is_err());
    assert!(validate_provider(&provider("safe", "https://api.example.com?token=secret", "key")).is_err());
    assert!(validate_provider(&provider("safe", "https://api.example.com", "env:BAD-NAME")).is_err());
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
