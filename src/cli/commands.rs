use crate::cli::args::{BackendAction, BackendDeviceAction, BackendPairAction, CliArgs, Commands, ProxyAction, RemoteBackendAction, ServiceAction};
use crate::core::agent_configuration::AgentConfiguration;
use crate::core::config::ConfigManager;
use crate::core::models::{AppType, SwitchMode};
use anyhow::{Context, Result};
use clap::CommandFactory;
use std::path::PathBuf;

fn mask_key(key: &str) -> String {
    let chars = key.chars().collect::<Vec<_>>();
    if key.starts_with("env:") || chars.len() <= 8 {
        key.to_string()
    } else {
        format!("{}...{}", chars[..4].iter().collect::<String>(), chars[chars.len() - 4..].iter().collect::<String>())
    }
}

fn get_db_path() -> PathBuf {
    crate::core::config::db_path()
}

pub fn run_cli(args: CliArgs) -> Result<()> {
    let command = match args.command {
        Some(cmd) => cmd,
        None => {
            // No subcommand — TUI is launched from main.rs, this branch is unreachable
            eprintln!("Usage: akmux <command>. Run 'akmux' without arguments to launch TUI.");
            std::process::exit(0);
        }
    };

    // Handle completions and man page generation before opening the database —
    // these are pure CLI introspection commands that don't need a DB connection.
    // This also ensures they work inside Nix build sandboxes where $HOME is
    // /homeless-shelter and the database directory cannot be created.
    match &command {
        Commands::Completions { shell } => return handle_completions(shell),
        Commands::Man => return handle_man(),
        _ => {}
    }

    let db_path = get_db_path();
    let defaults_path: Option<&std::path::Path> = None;
    let mgr = ConfigManager::new(&db_path, defaults_path)?;
    if let Err(error) = AgentConfiguration::new(&mgr).reconcile() {
        tracing::warn!("Failed to reconcile Agent configuration: {error:#}");
    }

    match command {
        Commands::Switch { target, local: _, proxy } => {
            let mode = if proxy { SwitchMode::Proxy } else { SwitchMode::Local };
            handle_switch(&mgr, target, mode)?;
        }
        Commands::List { providers, profiles } => {
            handle_list(&mgr, providers, profiles)?;
        }
        Commands::Add { what, provider } => {
            handle_add(&mgr, &what, provider.as_deref())?;
        }
        Commands::Edit { target } => {
            handle_edit(&mgr, &target)?;
        }
        Commands::Remove { target } => {
            handle_remove(&mgr, &target)?;
        }
        Commands::Proxy { action } => {
            handle_proxy(action)?;
        }
        Commands::Service { action } => {
            handle_service(action)?;
        }
        Commands::Backend { action } => {
            handle_backend(&mgr, action)?;
        }
        Commands::Usage { range, profile } => {
            handle_usage(&mgr, &range, profile.as_deref())?;
        }
        Commands::History { project, search } => {
            handle_history(&mgr, project.as_deref(), search.as_deref())?;
        }
        // Completions and Man are handled before DB init — see run_cli()
        Commands::Completions { .. } | Commands::Man => unreachable!(),
    }
    Ok(())
}

fn handle_backend(mgr: &ConfigManager, action: BackendAction) -> Result<()> {
    match action {
        BackendAction::Remote { action } => handle_remote_backend(mgr, action),
        BackendAction::Device { action } => handle_backend_device(mgr, action),
        BackendAction::Pair { action } => handle_backend_pair(action),
        BackendAction::Diagnostics => handle_backend_diagnostics(mgr),
        BackendAction::Audit { limit } => handle_backend_audit(mgr, limit),
    }
}

fn handle_backend_pair(action: BackendPairAction) -> Result<()> {
    let client = crate::session_service::admin::LocalAdminClient::from_env();
    let runtime = tokio::runtime::Runtime::new()?;
    runtime.block_on(async move {
        match action {
            BackendPairAction::Create => {
                let offer = client.create_pairing().await?;
                println!("Pairing ID: {}", offer.id);
                println!("Expires at: {}", offer.expires_at_ms);
                if let Ok(code) = qrcode::QrCode::new(offer.deep_link.as_bytes()) {
                    println!("{}", code.render::<qrcode::render::unicode::Dense1x2>().quiet_zone(true).build());
                }
                println!("Deep link: {}", offer.deep_link);
            }
            BackendPairAction::Pending => {
                for pairing in client.pending_pairings().await? {
                    println!(
                        "{}\t{}\t{}\t{}",
                        pairing.id,
                        pairing.device_name.as_deref().unwrap_or("waiting for device"),
                        pairing.source.as_deref().unwrap_or("waiting for request"),
                        pairing.expires_at_ms
                    );
                }
            }
            BackendPairAction::Confirm { id } => {
                client.confirm_pairing(&id).await?;
                println!("Pairing approved: {id}");
            }
            BackendPairAction::Cancel { id } => {
                client.cancel_pairing(&id).await?;
                println!("Pairing cancelled: {id}");
            }
        }
        Ok(())
    })
}

fn handle_remote_backend(mgr: &ConfigManager, action: RemoteBackendAction) -> Result<()> {
    match action {
        RemoteBackendAction::Configure {
            bind,
            public_url,
            allow_wildcard_bind,
        } => {
            let bind = bind.parse::<std::net::SocketAddr>().context("Remote bind must be an IP address and port")?;
            crate::session_service::remote::validate_bind(bind.ip(), allow_wildcard_bind)?;
            let public_url = crate::session_service::remote::validate_public_url(&public_url).context("Remote public URL is invalid")?;
            mgr.set_setting("remote.bind", &bind.to_string())?;
            mgr.set_setting("remote.public_url", public_url.as_str())?;
            mgr.set_setting("remote.allow_wildcard_bind", &allow_wildcard_bind.to_string())?;
            println!("Remote backend configured. A running daemon will apply the change automatically.");
            Ok(())
        }
        RemoteBackendAction::Enable => {
            anyhow::ensure!(mgr.db().has_active_backend_device()?, "Create at least one device credential before enabling Remote");
            let config = crate::session_service::remote::RemoteBackendConfig::load(mgr.db())?;
            anyhow::ensure!(config.public_url.is_some(), "Configure Remote before enabling it");
            crate::session_service::remote::validate_bind(config.bind.ip(), config.allow_wildcard_bind)?;
            mgr.set_setting("remote.enabled", "true")?;
            println!("Remote backend enabled.");
            Ok(())
        }
        RemoteBackendAction::Disable => {
            mgr.set_setting("remote.enabled", "false")?;
            println!("Remote backend disabled without stopping managed sessions.");
            Ok(())
        }
        RemoteBackendAction::Status => {
            let config = crate::session_service::remote::RemoteBackendConfig::load(mgr.db())?;
            println!("Remote backend: {}", if config.enabled { "enabled" } else { "disabled" });
            println!("Bind: {}", config.bind);
            println!("Public URL: {}", config.public_url.map_or_else(|| "not configured".into(), |url| url.to_string()));
            println!("Wildcard safeguard: {}", if config.allow_wildcard_bind { "explicitly allowed" } else { "blocked" });
            println!(
                "Active devices: {}",
                mgr.db().list_backend_devices()?.iter().filter(|device| device.revoked_at_ms.is_none()).count()
            );
            Ok(())
        }
    }
}

fn handle_backend_diagnostics(mgr: &ConfigManager) -> Result<()> {
    let config = crate::session_service::remote::RemoteBackendConfig::load(mgr.db())?;
    println!("Local listener: {}", if crate::session_service::control::is_running() { "running" } else { "stopped" });
    println!("Remote configured: {}", if config.public_url.is_some() { "yes" } else { "no" });
    println!("Remote enabled: {}", if config.enabled { "yes" } else { "no" });
    println!("Remote bind: {}", config.bind);
    match config.listener(mgr.db()) {
        Ok(Some(listener)) => {
            let reachable = std::net::TcpStream::connect_timeout(&listener.bind, std::time::Duration::from_millis(500)).is_ok();
            println!("Remote listener: {}", if reachable { "reachable" } else { "not reachable" });
            println!("Public URL: {}", listener.public_url);
            let health_url = listener.public_url.join("healthz")?;
            let public_health = tokio::runtime::Runtime::new()?.block_on(async {
                let client = reqwest::Client::builder()
                    .connect_timeout(std::time::Duration::from_secs(2))
                    .timeout(std::time::Duration::from_secs(5))
                    .build()?;
                let response = client.get(health_url).send().await?;
                anyhow::ensure!(response.status().is_success(), "HTTP {}", response.status());
                let body = response.json::<serde_json::Value>().await?;
                anyhow::ensure!(body.get("status").and_then(|value| value.as_str()) == Some("ok"), "unexpected health response");
                Ok::<_, anyhow::Error>(())
            });
            match public_health {
                Ok(()) => println!("Public HTTPS endpoint: healthy (TLS, proxy route, and Host validation passed)"),
                Err(error) => println!("Public HTTPS endpoint: unavailable ({error})"),
            }
        }
        Ok(None) => println!("Remote listener: disabled"),
        Err(error) => println!("Remote listener: invalid configuration ({error})"),
    }
    println!(
        "Active devices: {}",
        mgr.db().list_backend_devices()?.iter().filter(|device| device.revoked_at_ms.is_none()).count()
    );
    Ok(())
}

fn handle_backend_audit(mgr: &ConfigManager, limit: usize) -> Result<()> {
    for entry in mgr.db().list_backend_audit(limit)? {
        println!(
            "{}\t{}\t{}\t{}",
            entry.created_at_ms,
            entry.event,
            entry.device_id.as_deref().unwrap_or("-"),
            entry.source.as_deref().unwrap_or("-")
        );
    }
    Ok(())
}

fn handle_backend_device(mgr: &ConfigManager, action: BackendDeviceAction) -> Result<()> {
    use std::io::IsTerminal;

    let security = crate::session_service::remote::RemoteSecurity::load_or_create(&crate::core::config::data_dir().join("remote-auth.pepper"))?;
    match action {
        BackendDeviceAction::Create { name, show_token } => {
            anyhow::ensure!(show_token, "Device creation requires --show-token because the credential can only be displayed once");
            anyhow::ensure!(std::io::stdout().is_terminal(), "Refusing to print a device credential to non-interactive output");
            let (device, token) = security.create_device(mgr.db(), &name)?;
            println!("Device created: {} ({})", device.name, device.token_id);
            println!("Token (shown once): {token}");
            Ok(())
        }
        BackendDeviceAction::List => {
            for device in mgr.db().list_backend_devices()? {
                let state = if device.revoked_at_ms.is_some() { "revoked" } else { "active" };
                println!("{}\t{}\t{}", device.token_id, state, device.name);
            }
            Ok(())
        }
        BackendDeviceAction::Revoke { token_id } => {
            if crate::session_service::control::is_running() {
                let runtime = tokio::runtime::Runtime::new()?;
                let client = crate::session_service::admin::LocalAdminClient::from_env();
                runtime.block_on(client.revoke_device(&token_id))?;
            } else {
                let now = crate::session_service::remote::now_ms();
                anyhow::ensure!(mgr.db().revoke_backend_device(&token_id, now)?, "Active device not found: {token_id}");
                mgr.db().record_backend_audit("device.revoked", Some(&token_id), Some("cli"), now)?;
            }
            println!("Device revoked: {token_id}");
            Ok(())
        }
    }
}

fn handle_switch(mgr: &ConfigManager, target: Option<String>, mode: SwitchMode) -> Result<()> {
    let target = target.map_or_else(
        || {
            let providers = mgr.list_providers().unwrap_or_default();
            for p in &providers {
                for pr in &p.profiles {
                    if pr.default {
                        return Ok::<_, anyhow::Error>(format!("{}/{}", p.id, pr.id));
                    }
                }
            }
            anyhow::bail!("No target specified and no default profile found. Use 'akmux switch <provider>/<profile>'.");
        },
        Ok,
    )?;

    let (provider_id, profile_id) = target.split_once('/').with_context(|| format!("Invalid target '{}'. Use provider_id/profile_id", target))?;

    let config = AgentConfiguration::new(mgr).apply_claude_profile(provider_id, profile_id, mode)?;
    println!("Switched to: {} / {}", config.provider_name, config.profile_name);
    println!("  Opus:      {}", config.opus);
    println!("  Sonnet:    {}", config.sonnet);
    println!("  Haiku:     {}", config.haiku);
    println!("  Subagent:  {}", config.subagent);
    println!("  Mode:      {:?}", mode);
    Ok(())
}

fn handle_list(mgr: &ConfigManager, providers_only: bool, profiles_only: bool) -> Result<()> {
    if providers_only && profiles_only {
        anyhow::bail!("--providers and --profiles cannot be used together");
    }
    let providers = mgr.list_providers()?;
    for p in &providers {
        if !profiles_only {
            let source_icon = if p.source.can_delete() { "👤" } else { "🔒" };
            let default_marker = if p.profiles.iter().any(|pr| pr.default) { " ★" } else { "" };
            println!("{} {} ({}) [{}]{}", source_icon, p.name, p.id, p.api_url, default_marker);
        }
        if !providers_only {
            for pr in &p.profiles {
                let active = if pr.default { " (default)" } else { "" };
                let prefix = if profiles_only { format!("{}/{}", p.id, pr.id) } else { pr.id.clone() };
                println!("  ├─ {} ({}) [opus={}]{}", pr.name, prefix, pr.opus, active);
            }
        }
        if !profiles_only {
            println!();
        }
    }
    Ok(())
}

fn handle_add(mgr: &ConfigManager, what: &str, parent_provider: Option<&str>) -> Result<()> {
    match what {
        "provider" => {
            use dialoguer::{Input, Password};
            let id: String = Input::new().with_prompt("Provider ID").interact_text()?;
            let name: String = Input::new().with_prompt("Name").interact_text()?;
            let api_url: String = Input::new().with_prompt("API URL").interact_text()?;
            let api_key: String = Password::new().with_prompt("API Key (or env:VAR)").interact()?;
            let p = crate::core::models::Provider {
                id,
                name,
                api_url,
                api_key,
                codex_catalog: Default::default(),
                profiles: vec![],
                models: vec![],
                source: crate::core::models::Source::User,
            };
            crate::core::models::validate_provider(&p)?;
            if mgr.list_providers()?.iter().any(|provider| provider.id == p.id) {
                anyhow::bail!("Provider '{}' already exists", p.id);
            }
            AgentConfiguration::new(mgr).save_provider(AppType::Claude, &p)?;
            println!("Provider added.");
        }
        "profile" => {
            let provider_id = parent_provider.context("Usage: akmux add profile <provider_id>")?;
            // Ensure provider exists
            let providers = mgr.list_providers()?;
            let provider = providers
                .iter()
                .find(|p| p.id == provider_id)
                .with_context(|| format!("Provider '{}' not found. Create it first: akmux add provider", provider_id))?;
            use dialoguer::Input;
            let id: String = Input::new().with_prompt("Profile ID").interact_text()?;
            let name: String = Input::new().with_prompt("Name").interact_text()?;
            let opus: String = Input::new().with_prompt("Opus model").interact_text()?;
            let sonnet: String = Input::new().with_prompt("Sonnet model").interact_text()?;
            let haiku: String = Input::new().with_prompt("Haiku model").interact_text()?;
            let subagent: String = Input::new().with_prompt("Subagent model").interact_text()?;
            let pr = crate::core::models::Profile {
                id,
                name,
                opus,
                sonnet,
                haiku,
                subagent,
                default: false,
                source: crate::core::models::Source::User,
            };
            crate::core::models::validate_profile(&pr)?;
            if provider.profiles.iter().any(|profile| profile.id == pr.id) {
                anyhow::bail!("Profile '{}/{}' already exists", provider_id, pr.id);
            }
            AgentConfiguration::new(mgr).save_profile(provider_id, &pr)?;
            println!("Profile added to provider '{}'.", provider.name);
        }
        _ => anyhow::bail!("Usage: akmux add <provider|profile> [parent_provider]"),
    }
    Ok(())
}

fn handle_edit(mgr: &ConfigManager, target: &str) -> Result<()> {
    println!("Editing {} (interactive edit — launch TUI for full edit, or use add/remove)", target);
    // For CLI: just print current state; TUI provides full edit
    let providers = mgr.list_providers()?;
    let mut found = false;
    if let Some((provider_id, profile_id)) = target.split_once('/') {
        if let Some((p, pr)) = mgr.find_profile(provider_id, profile_id)? {
            found = true;
            println!("Provider: {} ({})", p.name, p.id);
            println!("Profile:  {} ({})", pr.name, pr.id);
            println!("  opus={} sonnet={} haiku={} subagent={}", pr.opus, pr.sonnet, pr.haiku, pr.subagent);
        }
    } else {
        for p in &providers {
            if p.id == target {
                found = true;
                println!("Provider: {} ({})", p.name, p.id);
                let masked_key = mask_key(&p.api_key);
                println!("  URL: {}  Key: {}", p.api_url, masked_key);
            }
        }
    }
    if !found {
        anyhow::bail!("Configuration '{}' not found", target);
    }
    Ok(())
}

fn handle_remove(mgr: &ConfigManager, target: &str) -> Result<()> {
    if let Some((provider_id, profile_id)) = target.split_once('/') {
        let (_, profile) = mgr.find_profile(provider_id, profile_id)?.with_context(|| format!("Profile '{}' not found", target))?;
        if !profile.source.can_delete() {
            anyhow::bail!("Cannot delete system default profile '{}'", target);
        }
        AgentConfiguration::new(mgr).delete_profile(provider_id, profile_id)?;
        println!("Removed profile: {}", target);
    } else {
        let providers = mgr.list_providers()?;
        let provider = providers
            .iter()
            .find(|provider| provider.id == target)
            .with_context(|| format!("Provider '{}' not found", target))?;
        if !provider.source.can_delete() {
            anyhow::bail!("Cannot delete system default provider '{}'", target);
        }
        AgentConfiguration::new(mgr).delete_provider(AppType::Claude, target)?;
        println!("Removed provider: {}", target);
    }
    Ok(())
}

fn handle_service(action: ServiceAction) -> Result<()> {
    use crate::proxy::service;
    match action {
        ServiceAction::Install { system } => service::install_service(system)?,
        ServiceAction::Uninstall { system } => service::uninstall_service(system)?,
    }
    Ok(())
}

fn handle_proxy(action: ProxyAction) -> Result<()> {
    use crate::proxy::service;
    match action {
        ProxyAction::Start => service::start_proxy()?,
        ProxyAction::Stop => service::stop_proxy()?,
        ProxyAction::Status => service::proxy_status()?,
        ProxyAction::Serve => {
            let db_path = get_db_path();
            let defaults_path: Option<&std::path::Path> = None;
            let mgr = ConfigManager::new(&db_path, defaults_path)?;
            let port: u16 = mgr.db().get_setting("proxy_port").and_then(|s| s.parse().ok()).unwrap_or(15721);
            let server = crate::proxy::server::ProxyServer::new(mgr);
            let rt = tokio::runtime::Runtime::new()?;
            rt.block_on(server.serve(port))?;
        }
    }
    Ok(())
}

fn handle_usage(mgr: &ConfigManager, range: &str, profile: Option<&str>) -> Result<()> {
    if !matches!(range, "day" | "week" | "month" | "all") {
        anyhow::bail!("Invalid range '{}'. Use day, week, month, or all.", range);
    }
    let summaries = mgr.db().query_usage("claude", range)?;
    let filtered: Vec<_> = summaries.iter().filter(|summary| profile.map_or(true, |filter| summary.model.contains(filter))).collect();
    let total_tokens: i64 = filtered
        .iter()
        .map(|s| s.total_prompt + s.total_completion + s.total_cache_read + s.total_cache_create)
        .sum();
    let total_requests: i64 = filtered.iter().map(|summary| summary.request_count).sum();
    println!("Token Usage ({})", range);
    println!("{:<30} {:>10} {:>10} {:>8}", "Model", "Prompt", "Completion", "Reqs");
    println!("{}", "-".repeat(60));
    for s in filtered {
        println!("{:<30} {:>10} {:>10} {:>8}", s.model, s.total_prompt, s.total_completion, s.request_count);
    }
    println!("{}", "-".repeat(60));
    println!("Total: {} tokens across {} requests", total_tokens, total_requests);
    Ok(())
}

fn project_name(s: &crate::db::sessions::SessionRecord) -> Option<String> {
    std::path::Path::new(&s.project_path).file_name().map(|n| n.to_string_lossy().to_string())
}

fn handle_history(mgr: &ConfigManager, project: Option<&str>, search: Option<&str>) -> Result<()> {
    // Auto-import Claude Code sessions before listing
    match crate::core::native_history::NativeHistoryIngestion::new(mgr.db()).refresh_sessions(crate::agent::AgentKind::Claude, |_| {}) {
        Ok(report) if report.changed > 0 => eprintln!("Imported {} new session(s)", report.changed),
        Err(e) => eprintln!("Warning: failed to import sessions: {}", e),
        _ => {}
    }
    let sessions = mgr.db().query_sessions("claude", project, search, 200)?;
    println!("Session History");
    println!("{:<6} {:<40} {:<12} {:>8} {:>6} Profile", "Date", "Title", "Project", "Tokens", "Msgs");
    println!("{}", "-".repeat(100));
    for s in &sessions {
        let date = s.start_time.get(5..16).unwrap_or(&s.start_time); // "MM-DD HH:MM", fallback safety
        let raw = s.title.as_deref().unwrap_or(&s.id);
        let is_uuid = raw.len() >= 32 && raw.chars().filter(|c| *c == '-').count() >= 4;
        let title: String = if is_uuid { project_name(s).unwrap_or_else(|| raw.to_string()) } else { raw.to_string() };
        let title = title.chars().take(40).collect::<String>();
        let project_short = project_name(s).unwrap_or_default().chars().take(12).collect::<String>();
        let tokens = s.prompt_tokens + s.completion_tokens;
        let profile = s.profile_id.as_deref().unwrap_or("-");
        println!("{:<6} {:<40} {:<12} {:>8} {:>6} {}", date, title, project_short, tokens, s.message_count, profile);
    }
    Ok(())
}

fn handle_completions(shell: &str) -> Result<()> {
    use crate::cli::args::CliArgs;
    use clap_complete::{generate, shells};
    let mut cmd = CliArgs::command();
    match shell {
        "zsh" => generate(shells::Zsh, &mut cmd, "akmux", &mut std::io::stdout()),
        "bash" => generate(shells::Bash, &mut cmd, "akmux", &mut std::io::stdout()),
        "fish" => generate(shells::Fish, &mut cmd, "akmux", &mut std::io::stdout()),
        _ => anyhow::bail!("Unsupported shell: {}. Use zsh, bash, or fish.", shell),
    }
    Ok(())
}

fn handle_man() -> Result<()> {
    use crate::cli::args::CliArgs;
    use clap_mangen::Man;
    let cmd = CliArgs::command();
    let man = Man::new(cmd);
    man.render(&mut std::io::stdout())?;
    Ok(())
}
