use serde::{Deserialize, Serialize};
use std::collections::HashSet;

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
    pub codex_catalog: CodexCatalog,
    #[serde(default)]
    pub profiles: Vec<Profile>,
    #[serde(default)]
    pub models: Vec<CodexModel>,
    #[serde(skip)]
    pub source: Source,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CodexCatalog {
    #[default]
    BuiltIn,
    Custom,
}

impl CodexCatalog {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::BuiltIn => "built-in",
            Self::Custom => "custom",
        }
    }
}

impl std::str::FromStr for CodexCatalog {
    type Err = ();

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "built-in" => Ok(Self::BuiltIn),
            "custom" => Ok(Self::Custom),
            _ => Err(()),
        }
    }
}

fn default_context_window() -> u64 {
    128_000
}

fn default_effective_context_percent() -> u8 {
    95
}

fn default_reasoning_effort() -> String {
    "medium".into()
}

fn default_reasoning_efforts() -> Vec<String> {
    vec!["low".into(), "medium".into(), "high".into()]
}

fn default_input_modalities() -> Vec<String> {
    vec!["text".into()]
}

fn default_true() -> bool {
    true
}

fn default_verbosity() -> String {
    "low".into()
}

/// Model metadata used to generate Codex's custom model catalog.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodexModel {
    pub slug: String,
    pub display_name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default = "default_context_window")]
    pub context_window: u64,
    #[serde(default)]
    pub max_context_window: Option<u64>,
    #[serde(default = "default_effective_context_percent")]
    pub effective_context_window_percent: u8,
    #[serde(default = "default_reasoning_effort")]
    pub default_reasoning_effort: String,
    #[serde(default = "default_reasoning_efforts")]
    pub supported_reasoning_efforts: Vec<String>,
    #[serde(default = "default_input_modalities")]
    pub input_modalities: Vec<String>,
    #[serde(default = "default_true")]
    pub supports_parallel_tool_calls: bool,
    #[serde(default = "default_true")]
    pub support_verbosity: bool,
    #[serde(default = "default_verbosity")]
    pub default_verbosity: String,
    #[serde(default)]
    pub supports_search_tool: bool,
    #[serde(default)]
    pub default: bool,
    #[serde(skip)]
    pub source: Source,
}

pub fn validate_codex_model(model: &CodexModel) -> anyhow::Result<()> {
    validate_text("Model slug", &model.slug, 256)?;
    validate_text("Model display name", &model.display_name, 100)?;
    if !model.description.is_empty() {
        validate_text("Model description", &model.description, 500)?;
    }
    if model.context_window == 0 {
        anyhow::bail!("Context window must be greater than zero");
    }
    if model.context_window > i64::MAX as u64 || model.max_context_window.is_some_and(|value| value > i64::MAX as u64) {
        anyhow::bail!("Context windows exceed the supported storage range");
    }
    if model.max_context_window.is_some_and(|max| max < model.context_window) {
        anyhow::bail!("Maximum context window must not be smaller than context window");
    }
    if !(1..=100).contains(&model.effective_context_window_percent) {
        anyhow::bail!("Effective context window percent must be between 1 and 100");
    }
    let allowed_efforts = ["none", "minimal", "low", "medium", "high", "xhigh", "max", "ultra"];
    if model.supported_reasoning_efforts.is_empty() || model.supported_reasoning_efforts.iter().any(|effort| !allowed_efforts.contains(&effort.as_str())) {
        anyhow::bail!("Supported reasoning efforts contain an invalid value");
    }
    if model.supported_reasoning_efforts.iter().collect::<HashSet<_>>().len() != model.supported_reasoning_efforts.len() {
        anyhow::bail!("Supported reasoning efforts must not contain duplicates");
    }
    if !model.supported_reasoning_efforts.contains(&model.default_reasoning_effort) {
        anyhow::bail!("Default reasoning effort must be supported by the model");
    }
    if model.input_modalities.is_empty() || model.input_modalities.iter().any(|modality| !matches!(modality.as_str(), "text" | "image")) {
        anyhow::bail!("Input modalities must contain text and/or image");
    }
    if model.input_modalities.iter().collect::<HashSet<_>>().len() != model.input_modalities.len() {
        anyhow::bail!("Input modalities must not contain duplicates");
    }
    if !matches!(model.default_verbosity.as_str(), "low" | "medium" | "high") {
        anyhow::bail!("Default verbosity must be low, medium or high");
    }
    Ok(())
}

pub fn validate_codex_provider_models(provider: &Provider) -> anyhow::Result<()> {
    let mut slugs = HashSet::new();
    let mut default_count = 0usize;
    for model in &provider.models {
        validate_codex_model(model)?;
        if !slugs.insert(model.slug.as_str()) {
            anyhow::bail!("Provider '{}' contains duplicate model slug '{}'", provider.id, model.slug);
        }
        default_count += usize::from(model.default);
    }
    if default_count > 1 {
        anyhow::bail!("Provider '{}' has more than one default model", provider.id);
    }
    Ok(())
}

pub fn validate_provider(provider: &Provider) -> anyhow::Result<()> {
    validate_id("Provider ID", &provider.id)?;
    validate_text("Provider name", &provider.name, 100)?;
    let url = reqwest::Url::parse(provider.api_url.trim()).map_err(|error| anyhow::anyhow!("Invalid provider URL: {}", error))?;
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
        if variable.is_empty() || !variable.chars().all(|ch| ch == '_' || ch.is_ascii_alphanumeric()) {
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
    if !value.chars().all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.')) {
        anyhow::bail!("{} may only contain letters, numbers, '.', '_' and '-'", label);
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
