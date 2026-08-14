#[derive(Debug, thiserror::Error)]
#[error("API key unavailable for '{provider_id}'. Set {env_var} or use a literal key.")]
pub struct ApiKeyUnavailable {
    pub provider_id: String,
    pub env_var: String,
}

impl ApiKeyUnavailable {
    pub fn new(provider_id: &str, raw_key: &str, default_var: &str) -> Self {
        Self {
            provider_id: provider_id.to_string(),
            env_var: raw_key.strip_prefix("env:").unwrap_or(default_var).to_string(),
        }
    }
}

/// Parse an env:VAR_NAME reference and resolve the value.
/// - "env:FOO" → reads only $FOO (or the matching env-file entry)
/// - "literal-key" → returns as-is
/// - "" → reads $CLAUDE_API_KEY, then ""
pub fn resolve_api_key(raw: &str) -> String {
    resolve_with_default(raw, "CLAUDE_API_KEY")
}

/// Resolve a Codex/OpenAI key without ever falling back to a Claude key.
pub fn resolve_codex_api_key(raw: &str) -> String {
    resolve_with_default(raw, "OPENAI_API_KEY")
}

fn resolve_with_default(raw: &str, default_var: &str) -> String {
    if let Some(var_name) = raw.strip_prefix("env:") {
        lookup_env(var_name).unwrap_or_default()
    } else if raw.is_empty() {
        lookup_env(default_var).unwrap_or_default()
    } else {
        raw.to_string()
    }
}

fn lookup_env(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .filter(|value| !value.is_empty())
        .or_else(|| lookup_env_files(&crate::core::config::config_dir(), name))
}

fn lookup_env_files(config_dir: &std::path::Path, name: &str) -> Option<String> {
    let configured_path = std::env::var("CCSWITCH_ENV_FILE")
        .ok()
        .filter(|path| !path.trim().is_empty())
        .or_else(|| {
            std::fs::read_to_string(config_dir.join("env-path"))
                .ok()
                .map(|path| path.trim().to_string())
                .filter(|path| !path.is_empty())
        })
        .map(|path| expand_env_file_path(&path));
    let default_path = config_dir.join("env");

    configured_path
        .as_deref()
        .and_then(|path| read_env_file_value(path, name))
        .filter(|value| !value.is_empty())
        .or_else(|| {
            if configured_path.as_deref() == Some(default_path.as_path()) {
                None
            } else {
                read_env_file_value(&default_path, name).filter(|value| !value.is_empty())
            }
        })
}

fn expand_env_file_path(raw: &str) -> std::path::PathBuf {
    let raw = raw.strip_prefix('-').unwrap_or(raw);
    let Some(home) = dirs::home_dir() else {
        return raw.into();
    };
    if raw == "%h" || raw == "~" {
        return home;
    }
    if let Some(rest) = raw.strip_prefix("%h/").or_else(|| raw.strip_prefix("~/")) {
        return home.join(rest);
    }
    raw.into()
}

fn read_env_file_value(path: &std::path::Path, name: &str) -> Option<String> {
    let content = std::fs::read_to_string(path).ok()?;
    for raw_line in content.lines() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let line = line.strip_prefix("export ").unwrap_or(line).trim();
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        if key.trim() != name {
            continue;
        }
        let value = value.trim();
        let value = if value.len() >= 2 && ((value.starts_with('"') && value.ends_with('"')) || (value.starts_with('\'') && value.ends_with('\''))) {
            &value[1..value.len() - 1]
        } else {
            value
        };
        return Some(value.to_string());
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn test_resolve_literal_key() {
        let _guard = ENV_LOCK.lock().unwrap();
        let result = resolve_api_key("sk-abc123");
        assert_eq!(result, "sk-abc123");
    }

    #[test]
    fn test_resolve_env_ref() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::set_var("TEST_KEY", "test-value");
        let result = resolve_api_key("env:TEST_KEY");
        assert_eq!(result, "test-value");
        std::env::remove_var("TEST_KEY");
    }

    #[test]
    fn test_explicit_env_ref_does_not_use_unrelated_fallback() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::set_var("CLAUDE_API_KEY", "fallback-key");
        let result = resolve_api_key("env:NONEXISTENT_VAR");
        assert_eq!(result, "");
        std::env::remove_var("CLAUDE_API_KEY");
    }

    #[test]
    fn test_codex_empty_uses_openai_key() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::set_var("OPENAI_API_KEY", "openai-key");
        std::env::set_var("CLAUDE_API_KEY", "claude-key");
        assert_eq!(resolve_codex_api_key(""), "openai-key");
        std::env::remove_var("OPENAI_API_KEY");
        std::env::remove_var("CLAUDE_API_KEY");
    }

    #[test]
    fn test_resolve_empty_fallback() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::set_var("CLAUDE_API_KEY", "default-key");
        let result = resolve_api_key("");
        assert_eq!(result, "default-key");
        std::env::remove_var("CLAUDE_API_KEY");
    }

    #[test]
    fn test_read_env_file_value() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("env");
        std::fs::write(&path, "# comment\nexport CODEX_KEY=\"from-file\"\n").unwrap();
        assert_eq!(read_env_file_value(&path, "CODEX_KEY"), Some("from-file".into()));
    }

    #[test]
    fn test_api_key_unavailable_error_keeps_provider_and_variable() {
        let error = ApiKeyUnavailable::new("deepseek", "env:DEEPSEEK_API_KEY", "CLAUDE_API_KEY");
        assert_eq!(error.provider_id, "deepseek");
        assert_eq!(error.env_var, "DEEPSEEK_API_KEY");
        assert_eq!(error.to_string(), "API key unavailable for 'deepseek'. Set DEEPSEEK_API_KEY or use a literal key.");
    }

    #[test]
    fn test_configured_env_file_path() {
        let dir = tempfile::tempdir().unwrap();
        let secrets_path = dir.path().join("secrets.env");
        std::fs::write(&secrets_path, "CUSTOM_KEY=from-custom-file\n").unwrap();
        std::fs::write(dir.path().join("env-path"), format!("{}\n", secrets_path.display())).unwrap();

        assert_eq!(lookup_env_files(dir.path(), "CUSTOM_KEY"), Some("from-custom-file".into()));
    }

    #[test]
    fn test_configured_env_file_falls_back_to_default_file() {
        let dir = tempfile::tempdir().unwrap();
        let secrets_path = dir.path().join("secrets.env");
        std::fs::write(&secrets_path, "OTHER_KEY=other-value\n").unwrap();
        std::fs::write(dir.path().join("env"), "DEFAULT_KEY=default-value\n").unwrap();
        std::fs::write(dir.path().join("env-path"), format!("{}\n", secrets_path.display())).unwrap();

        assert_eq!(lookup_env_files(dir.path(), "DEFAULT_KEY"), Some("default-value".into()));
    }
}
