mod cli;
mod core;
mod db;
mod proxy;
mod tui;

use clap::Parser;
use cli::args::CliArgs;

fn main() {
    let args = CliArgs::parse();
    if args.command.is_none() {
        // First-launch check: import session data with progress before TUI starts.
        // Subsequent launches skip this — session_history already has data.
        pre_tui_import();

        // Init tracing to file in TUI mode — stderr output corrupts the terminal.
        let log_dir = ccswitch::core::config::data_dir();
        std::fs::create_dir_all(&log_dir).ok();
        let log_path = log_dir.join("ccs.log");
        let mut log_options = std::fs::OpenOptions::new();
        log_options.create(true).append(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            log_options.mode(0o600);
        }
        let file = match log_options.open(&log_path) {
            Ok(f) => {
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    if let Err(error) = f.set_permissions(std::fs::Permissions::from_mode(0o600)) {
                        eprintln!(
                            "Warning: failed to restrict log file permissions: {}",
                            error
                        );
                    }
                }
                f
            }
            Err(_) => match std::fs::File::create("/dev/null") {
                Ok(f) => f,
                Err(_) => {
                    eprintln!("Failed to open log file");
                    std::process::exit(1);
                }
            },
        };
        tracing_subscriber::fmt()
            .with_writer(std::sync::Mutex::new(file))
            .with_target(false)
            .init();

        // Launch TUI
        if let Err(e) = tui::run_tui() {
            eprintln!("TUI error: {}", e);
            std::process::exit(1);
        }
        return;
    }

    tracing_subscriber::fmt::init();
    if let Err(e) = cli::commands::run_cli(args) {
        eprintln!("Error: {}", e);
        std::process::exit(1);
    }
}

/// If this is the first launch (no session data yet), run session import
/// with a terminal progress bar before the TUI starts.
fn pre_tui_import() {
    let db_path = crate::core::config::db_path();

    let db = match db::Db::open(&db_path) {
        Ok(db) => db,
        Err(e) => {
            eprintln!("Warning: cannot open session DB: {}", e);
            return;
        }
    };

    // Check if this is first launch (for progress bar display)
    let is_first_launch: bool = db
        .conn()
        .query_row("SELECT COUNT(*) FROM session_history", [], |r| {
            r.get::<_, i64>(0)
        })
        .map(|c| c == 0)
        .unwrap_or(true);

    if is_first_launch {
        eprintln!("\n🔄 First launch: importing Claude Code sessions...\n");
    }

    // Always run import — incremental (mtime-based) on subsequent launches
    let result = crate::core::import::import_claude_sessions_with_progress(
        &db,
        |files_done, files_total, imported| {
            if is_first_launch {
                let pct = if files_total > 0 {
                    (files_done as f64 / files_total as f64 * 100.0) as usize
                } else {
                    0
                };
                let bar_len = (pct / 4).min(25);
                let bar = format!(
                    "{}{}",
                    "█".repeat(bar_len),
                    "░".repeat(25usize.saturating_sub(bar_len))
                );
                eprint!(
                    "\r  [{}] {:>3}%  {}/{} files  {} sessions imported",
                    bar, pct, files_done, files_total, imported
                );
                std::io::Write::flush(&mut std::io::stderr()).ok();
            }
        },
    );

    if is_first_launch {
        match result {
            Ok(n) => eprintln!("\n\n✅ Imported {} sessions. Launching CCSwitch...\n", n),
            Err(e) => eprintln!("\n\n⚠️  Import finished with errors: {}\n", e),
        }
    } else if let Err(e) = result {
        // Silent on subsequent launches unless there's an actual error
        eprintln!("⚠️  Session import error: {}", e);
    }
}
