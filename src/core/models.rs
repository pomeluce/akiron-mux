use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum AppType {
    #[default]
    Claude,
    Codex,
}

impl AppType {
    pub fn as_str(self) -> &'static str {
        match self {
            AppType::Claude => "claude",
            AppType::Codex => "codex",
        }
    }

    pub fn display_name(self) -> &'static str {
        match self {
            AppType::Claude => "Claude Code",
            AppType::Codex => "Codex",
        }
    }

    pub fn toggle(self) -> Self {
        match self {
            AppType::Claude => AppType::Codex,
            AppType::Codex => AppType::Claude,
        }
    }

    pub fn active_provider_key(self) -> &'static str {
        match self {
            AppType::Claude => "active_provider",
            AppType::Codex => "active_codex_provider",
        }
    }
}

impl std::str::FromStr for AppType {
    type Err = ();

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "claude" => Ok(AppType::Claude),
            "codex" => Ok(AppType::Codex),
            _ => Err(()),
        }
    }
}

/// Represents whether a config came from system defaults or user DB
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Source {
    #[default]
    #[serde(rename = "system")]
    System,
    #[serde(rename = "user")]
    User,
}

impl Source {
    pub fn can_delete(&self) -> bool {
        matches!(self, Source::User)
    }

    pub fn as_str(&self) -> &str {
        match self {
            Source::System => "system",
            Source::User => "user",
        }
    }
}

impl std::str::FromStr for Source {
    type Err = ();
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "system" => Ok(Source::System),
            "user" => Ok(Source::User),
            _ => Err(()),
        }
    }
}

/// An API provider (vendor)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Provider {
    pub id: String,
    pub name: String,
    pub api_url: String,
    pub api_key: String,
    #[serde(default)]
    pub profiles: Vec<Profile>,
    #[serde(skip)]
    pub source: Source,
}

pub fn validate_provider(provider: &Provider) -> anyhow::Result<()> {
    validate_id("Provider ID", &provider.id)?;
    validate_text("Provider name", &provider.name, 100)?;
    let url = reqwest::Url::parse(provider.api_url.trim())
        .map_err(|error| anyhow::anyhow!("Invalid provider URL: {}", error))?;
    if !matches!(url.scheme(), "http" | "https") {
        anyhow::bail!("Provider URL must use http or https");
    }
    if !url.username().is_empty() || url.password().is_some() {
        anyhow::bail!("Provider URL must not contain embedded credentials");
    }
    if url.query().is_some() || url.fragment().is_some() {
        anyhow::bail!("Provider URL must not contain a query string or fragment");
    }
    if provider.api_key.chars().any(char::is_control) {
        anyhow::bail!("API key must not contain control characters");
    }
    if provider.api_key.chars().count() > 4096 {
        anyhow::bail!("API key must not exceed 4096 characters");
    }
    if let Some(variable) = provider.api_key.strip_prefix("env:") {
        if variable.is_empty()
            || !variable
                .chars()
                .all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
        {
            anyhow::bail!("Environment variable references must use env:VARIABLE_NAME");
        }
    }
    Ok(())
}

/// A model configuration profile under a provider
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Profile {
    pub id: String,
    pub name: String,
    #[serde(alias = "reasoning_model")]
    pub opus: String,
    #[serde(default)]
    pub sonnet: String,
    #[serde(alias = "task_model")]
    pub haiku: String,
    #[serde(default)]
    pub subagent: String,
    #[serde(default)]
    pub default: bool,
    #[serde(skip)]
    pub source: Source,
}

pub fn validate_profile(profile: &Profile) -> anyhow::Result<()> {
    validate_id("Profile ID", &profile.id)?;
    validate_text("Profile name", &profile.name, 100)?;
    for (label, model) in [
        ("Opus model", &profile.opus),
        ("Sonnet model", &profile.sonnet),
        ("Haiku model", &profile.haiku),
        ("Subagent model", &profile.subagent),
    ] {
        validate_text(label, model, 256)?;
    }
    Ok(())
}

fn validate_id(label: &str, value: &str) -> anyhow::Result<()> {
    if value.is_empty() || value.len() > 64 {
        anyhow::bail!("{} must be 1-64 characters", label);
    }
    if !value
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.'))
    {
        anyhow::bail!(
            "{} may only contain letters, numbers, '.', '_' and '-'",
            label
        );
    }
    Ok(())
}

fn validate_text(label: &str, value: &str, max_chars: usize) -> anyhow::Result<()> {
    let trimmed = value.trim();
    if trimmed.is_empty() || trimmed.chars().count() > max_chars {
        anyhow::bail!("{} must be 1-{} characters", label, max_chars);
    }
    if value.chars().any(char::is_control) {
        anyhow::bail!("{} must not contain control characters", label);
    }
    Ok(())
}

/// The resolved active config (what gets applied)
#[derive(Debug, Clone)]
pub struct ActiveConfig {
    pub provider_id: String,
    pub profile_id: String,
    pub provider_name: String,
    pub profile_name: String,
    pub base_url: String,
    pub auth_token: String,
    pub opus: String,
    pub sonnet: String,
    pub haiku: String,
    pub subagent: String,
}

/// How the switch should be applied
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SwitchMode {
    Local,
    Proxy,
}
