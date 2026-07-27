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
        .or_else(|| {
            read_env_file_value(&crate::core::config::config_dir().join("env"), name)
                .filter(|value| !value.is_empty())
        })
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
        let value = if value.len() >= 2
            && ((value.starts_with('"') && value.ends_with('"'))
                || (value.starts_with('\'') && value.ends_with('\'')))
        {
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
        assert_eq!(
            read_env_file_value(&path, "CODEX_KEY"),
            Some("from-file".into())
        );
    }
}
