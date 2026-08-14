use crate::core::models::{validate_codex_provider_models, CodexCatalog, CodexModel, Provider};
use anyhow::{Context, Result};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

const BASE_INSTRUCTIONS: &str = r#"You are Codex, a coding agent. You and the user share a workspace and collaborate until their software task is genuinely complete.

Work directly in the repository when the user asks for implementation. Inspect relevant code before editing, preserve unrelated user changes, and prefer small, maintainable changes that follow the project's existing conventions. Use available tools to search, edit, build, test, and diagnose. Never claim a command succeeded unless it was actually run successfully.

Keep the user informed with concise progress updates while working. Lead the final response with the outcome, mention important files and verification, and clearly disclose anything that could not be completed. For reviews, prioritize concrete bugs, regressions, security risks, and missing tests. Be careful with credentials and destructive operations, and request approval when an action could cause material data loss or exceed the user's stated scope."#;

pub fn default_catalog_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    Path::new(&home).join(".codex/ccswitch/models.json")
}

pub fn build_catalog(providers: &[Provider]) -> Result<Value> {
    let mut models = BTreeMap::<String, Value>::new();
    for provider in providers.iter().filter(|provider| provider.codex_catalog == CodexCatalog::Custom) {
        validate_codex_provider_models(provider)?;
        for model in &provider.models {
            let entry = model_entry(model);
            if let Some(existing) = models.get(&model.slug) {
                if existing != &entry {
                    anyhow::bail!("Model slug '{}' has conflicting definitions across providers", model.slug);
                }
            } else {
                models.insert(model.slug.clone(), entry);
            }
        }
    }
    Ok(json!({ "models": models.into_values().collect::<Vec<_>>() }))
}

pub fn write_catalog(path: &Path, providers: &[Provider]) -> Result<()> {
    let catalog = build_catalog(providers)?;
    write_private_json(path, &catalog)
}

pub fn catalog_status(path: &Path, providers: &[Provider]) -> Result<bool> {
    if !path.exists() {
        return Ok(false);
    }
    let existing: Value = serde_json::from_str(&std::fs::read_to_string(path)?).context("Failed to parse CCSwitch models.json")?;
    Ok(existing == build_catalog(providers)?)
}

pub fn model_entry(model: &CodexModel) -> Value {
    let efforts = model
        .supported_reasoning_efforts
        .iter()
        .map(|effort| {
            json!({
                "effort": effort,
                "description": reasoning_description(effort),
            })
        })
        .collect::<Vec<_>>();
    let supports_images = model.input_modalities.iter().any(|item| item == "image");

    json!({
        "slug": model.slug,
        "display_name": model.display_name,
        "description": if model.description.is_empty() { "Third-party Responses API model." } else { &model.description },
        "default_reasoning_level": model.default_reasoning_effort,
        "supported_reasoning_levels": efforts,
        "shell_type": "shell_command",
        "visibility": "list",
        "supported_in_api": true,
        "priority": 1,
        "availability_nux": null,
        "upgrade": null,
        "include_skills_usage_instructions": false,
        "default_reasoning_summary": "none",
        "support_verbosity": model.support_verbosity,
        "default_verbosity": if model.support_verbosity { Value::String(model.default_verbosity.clone()) } else { Value::Null },
        "apply_patch_tool_type": "freeform",
        "web_search_tool_type": "text",
        "truncation_policy": { "mode": "tokens", "limit": 10000 },
        "supports_parallel_tool_calls": model.supports_parallel_tool_calls,
        "supports_image_detail_original": supports_images,
        "context_window": model.context_window,
        "max_context_window": model.max_context_window.unwrap_or(model.context_window),
        "comp_hash": "3000",
        "effective_context_window_percent": model.effective_context_window_percent,
        "experimental_supported_tools": [],
        "input_modalities": model.input_modalities,
        "supports_search_tool": model.supports_search_tool,
        "use_responses_lite": false,
        "tool_mode": null,
        "multi_agent_version": "v2",
        "model_messages": null,
        "base_instructions": BASE_INSTRUCTIONS,
    })
}

fn reasoning_description(effort: &str) -> &'static str {
    match effort {
        "none" => "No additional reasoning",
        "minimal" => "Minimal reasoning for the fastest responses",
        "low" => "Fast responses with lighter reasoning",
        "medium" => "Balances speed and reasoning depth",
        "high" => "Greater reasoning depth for complex tasks",
        "xhigh" => "Extra high reasoning depth for complex tasks",
        "max" => "Maximum reasoning depth for the hardest tasks",
        "ultra" => "Maximum reasoning with automatic task delegation",
        _ => "Model reasoning effort",
    }
}

fn write_private_json(path: &Path, value: &Value) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let file_name = path.file_name().and_then(|name| name.to_str()).unwrap_or("models.json");
    let temporary = path.with_file_name(format!(".{}.ccswitch.tmp", file_name));
    let mut options = std::fs::OpenOptions::new();
    options.create(true).truncate(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&temporary)?;
    serde_json::to_writer_pretty(&mut file, value)?;
    file.sync_all()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&temporary, std::fs::Permissions::from_mode(0o600))?;
    }
    #[cfg(windows)]
    if path.exists() {
        std::fs::remove_file(path)?;
    }
    std::fs::rename(temporary, path)?;
    Ok(())
}
