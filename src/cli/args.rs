use clap::{Parser, Subcommand};

/// AkironMux — unified Claude Code and Codex configuration manager
#[derive(Parser, Debug)]
#[command(name = "akmux", version, about, long_about = None)]
pub struct CliArgs {
    #[command(subcommand)]
    pub command: Option<Commands>,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Switch to a model profile (usage: akmux switch <provider>/<profile>)
    Switch {
        /// Target profile as "provider_id/profile_id" or just "profile_id"
        target: Option<String>,
        /// Use local mode (modify settings.json directly)
        #[arg(long, conflicts_with = "proxy")]
        local: bool,
        /// Use proxy mode
        #[arg(long)]
        proxy: bool,
    },

    /// List providers and profiles
    List {
        /// Only list providers
        #[arg(long)]
        providers: bool,
        /// Only list profiles
        #[arg(long)]
        profiles: bool,
    },

    /// Add a provider or profile interactively
    Add {
        /// What to add
        what: String,
        /// Parent provider (when adding a profile)
        provider: Option<String>,
    },

    /// Edit a provider or profile
    Edit {
        /// "provider_id" or "provider_id/profile_id"
        target: String,
    },

    /// Remove a provider or profile (user-added only)
    Remove {
        /// "provider_id" or "provider_id/profile_id"
        target: String,
    },

    /// Manage the proxy service
    Proxy {
        #[command(subcommand)]
        action: ProxyAction,
    },

    /// Install / uninstall background service
    Service {
        #[command(subcommand)]
        action: ServiceAction,
    },

    /// Show token usage statistics
    Usage {
        #[arg(long, default_value = "week")]
        range: String,
        #[arg(long)]
        profile: Option<String>,
    },

    /// Show session history
    History {
        #[arg(long)]
        project: Option<String>,
        #[arg(long)]
        search: Option<String>,
    },

    /// Generate shell completions
    Completions {
        /// Shell: zsh, bash, fish
        shell: String,
    },

    /// Output man page (roff format)
    Man,
}

#[derive(Subcommand, Debug)]
pub enum ProxyAction {
    /// Start proxy in background
    Start,
    /// Stop the running proxy
    Stop,
    /// Show proxy status
    Status,
    /// Run proxy in foreground (debugging)
    Serve,
}

#[derive(Subcommand, Debug)]
pub enum ServiceAction {
    /// Install background service (default: user-level)
    Install {
        /// Install as system-level service (requires root)
        #[arg(long)]
        system: bool,
    },
    /// Uninstall background service
    Uninstall {
        /// Uninstall system-level service (requires root)
        #[arg(long)]
        system: bool,
    },
}
