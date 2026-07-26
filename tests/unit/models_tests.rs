use ccswitch::core::models::{Provider, Profile, Source};

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
