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

    /// Configure authenticated Local/Remote session backends
    Backend {
        #[command(subcommand)]
        action: BackendAction,
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
pub enum BackendAction {
    /// Configure and control the authenticated Remote listener
    Remote {
        #[command(subcommand)]
        action: RemoteBackendAction,
    },
    /// Create, list, and revoke Remote client devices
    Device {
        #[command(subcommand)]
        action: BackendDeviceAction,
    },
    /// Create and approve short-lived mobile pairing requests
    Pair {
        #[command(subcommand)]
        action: BackendPairAction,
    },
    /// Check listener, configuration, and public endpoint health
    Diagnostics,
    /// Inspect recent security audit metadata (never credential or terminal content)
    Audit {
        #[arg(long, default_value_t = 50)]
        limit: usize,
    },
}

#[derive(Subcommand, Debug)]
pub enum BackendPairAction {
    /// Create a 60-second pairing deep link
    Create,
    /// List pending pairing requests
    Pending,
    /// Approve a pending pairing request
    Confirm { id: String },
    /// Cancel a pending pairing request immediately
    Cancel { id: String },
}

#[derive(Subcommand, Debug)]
pub enum RemoteBackendAction {
    /// Save the private bind address and public HTTPS URL
    Configure {
        #[arg(long, default_value = "127.0.0.1:17322")]
        bind: String,
        #[arg(long)]
        public_url: String,
        /// Explicitly permit 0.0.0.0/[::]; requires external firewall or TLS proxy safeguards
        #[arg(long)]
        allow_wildcard_bind: bool,
    },
    /// Enable Remote on the next daemon start
    Enable,
    /// Disable Remote on the next daemon start
    Disable,
    /// Show Remote configuration without credentials
    Status,
}

#[derive(Subcommand, Debug)]
pub enum BackendDeviceAction {
    /// Create a device credential and print it exactly once
    Create {
        #[arg(long)]
        name: String,
        #[arg(long)]
        show_token: bool,
    },
    /// List devices without credential material
    List,
    /// Revoke a device immediately
    Revoke { token_id: String },
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
