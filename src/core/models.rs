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
