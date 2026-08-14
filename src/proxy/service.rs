use std::path::PathBuf;
use std::process::Command;

use anyhow::{Context, Result};

const SERVICE_NAME: &str = "ccs-proxy";

// ── Linux systemd ──────────────────────────────────────────────────

#[cfg(target_os = "linux")]
pub fn install_service(system: bool) -> Result<()> {
    let exe = std::env::current_exe()?;
    let unit = format!(
        r#"[Unit]
Description=CCSwitch Proxy Server
After=network.target

[Service]
ExecStart={} proxy serve
Restart=on-failure
RestartSec=5

[Install]
WantedBy=default.target
"#,
        systemd_quote(&exe)
    );

    if system {
        let path = PathBuf::from("/etc/systemd/system").join(format!("{}.service", SERVICE_NAME));
        std::fs::write(&path, unit).context("Failed to write systemd unit (need root?)")?;
        run_cmd("systemctl", &["daemon-reload"])?;
        run_cmd("systemctl", &["enable", SERVICE_NAME])?;
        run_cmd("systemctl", &["start", SERVICE_NAME])?;
        println!("Service installed (system): systemctl status {}", SERVICE_NAME);
    } else {
        let dir = systemd_user_dir();
        std::fs::create_dir_all(&dir)?;
        let path = dir.join(format!("{}.service", SERVICE_NAME));
        std::fs::write(&path, unit)?;
        run_cmd("systemctl", &["--user", "daemon-reload"])?;
        run_cmd("systemctl", &["--user", "enable", SERVICE_NAME])?;
        run_cmd("systemctl", &["--user", "start", SERVICE_NAME])?;
        println!("Service installed (user): systemctl --user status {}", SERVICE_NAME);
    }
    Ok(())
}

#[cfg(target_os = "linux")]
pub fn uninstall_service(system: bool) -> Result<()> {
    if system {
        run_cmd("systemctl", &["stop", SERVICE_NAME]).ok();
        run_cmd("systemctl", &["disable", SERVICE_NAME]).ok();
        let path = PathBuf::from("/etc/systemd/system").join(format!("{}.service", SERVICE_NAME));
        std::fs::remove_file(&path).ok();
        run_cmd("systemctl", &["daemon-reload"])?;
    } else {
        run_cmd("systemctl", &["--user", "stop", SERVICE_NAME]).ok();
        run_cmd("systemctl", &["--user", "disable", SERVICE_NAME]).ok();
        let path = systemd_user_dir().join(format!("{}.service", SERVICE_NAME));
        std::fs::remove_file(&path).ok();
        run_cmd("systemctl", &["--user", "daemon-reload"])?;
    }
    println!("Service uninstalled.");
    Ok(())
}

fn systemd_user_dir() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    PathBuf::from(home).join(".config").join("systemd").join("user")
}

// ── macOS launchd ──────────────────────────────────────────────────

#[cfg(target_os = "macos")]
pub fn install_service(_system: bool) -> Result<()> {
    let exe = std::env::current_exe()?;
    let log_dir = crate::core::config::data_dir();
    std::fs::create_dir_all(&log_dir)?;
    let log_path = log_dir.join("ccs-proxy.log");
    let plist = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>com.ccswitch.{0}</string>
    <key>ProgramArguments</key>
    <array>
        <string>{1}</string>
        <string>proxy</string>
        <string>serve</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <true/>
    <key>StandardOutPath</key>
    <string>{2}</string>
    <key>StandardErrorPath</key>
    <string>{2}</string>
</dict>
</plist>"#,
        SERVICE_NAME,
        xml_escape(&exe.to_string_lossy()),
        xml_escape(&log_path.to_string_lossy()),
    );

    let dir = launchd_agent_dir()?;
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(format!("com.ccswitch.{}.plist", SERVICE_NAME));
    std::fs::write(&path, plist)?;

    let uid = get_uid()?;
    // Unload if already loaded, then bootstrap
    let domain = format!("gui/{}", uid);
    run_cmd("launchctl", &["bootout", &domain, &path.display().to_string()]).ok();
    run_cmd("launchctl", &["bootstrap", &domain, &path.display().to_string()])?;

    println!("Service installed: launchctl list | grep ccswitch");
    Ok(())
}

#[cfg(target_os = "macos")]
pub fn uninstall_service(_system: bool) -> Result<()> {
    let dir = launchd_agent_dir()?;
    let path = dir.join(format!("com.ccswitch.{}.plist", SERVICE_NAME));

    let uid = get_uid().unwrap_or(0);
    let domain = format!("gui/{}", uid);
    run_cmd("launchctl", &["bootout", &domain, &path.display().to_string()]).ok();
    std::fs::remove_file(&path).ok();
    println!("Service uninstalled.");
    Ok(())
}

#[cfg(target_os = "macos")]
fn get_uid() -> Result<u32> {
    let output = Command::new("id").arg("-u").output()?;
    let s = String::from_utf8_lossy(&output.stdout);
    s.trim().parse().context("Failed to parse uid")
}

#[cfg(target_os = "macos")]
fn launchd_agent_dir() -> Result<PathBuf> {
    let home = std::env::var("HOME").context("HOME not set")?;
    Ok(PathBuf::from(home).join("Library").join("LaunchAgents"))
}

#[cfg(target_os = "macos")]
fn xml_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

// ── Windows Schtasks ───────────────────────────────────────────────

#[cfg(target_os = "windows")]
pub fn install_service(_system: bool) -> Result<()> {
    let exe = std::env::current_exe()?;
    let task_name = format!("{}-service", SERVICE_NAME);

    // Remove existing task first
    run_cmd("schtasks", &["/delete", "/tn", &task_name, "/f"]).ok();

    // Create scheduled task: runs at user logon, hidden, with restart on failure
    run_cmd(
        "schtasks",
        &[
            "/create",
            "/tn",
            &task_name,
            "/tr",
            &format!("\"{}\" proxy serve", exe.display()),
            "/sc",
            "onlogon",
            "/rl",
            "highest",
            "/f",
        ],
    )?;

    // Start it now
    run_cmd("schtasks", &["/run", "/tn", &task_name])?;

    println!("Service installed (Scheduled Task): schtasks /query /tn {}", task_name);
    Ok(())
}

#[cfg(target_os = "windows")]
pub fn uninstall_service(_system: bool) -> Result<()> {
    let task_name = format!("{}-service", SERVICE_NAME);
    run_cmd("schtasks", &["/end", "/tn", &task_name]).ok();
    run_cmd("schtasks", &["/delete", "/tn", &task_name, "/f"]).ok();
    println!("Service uninstalled.");
    Ok(())
}

// ── Shared helpers ─────────────────────────────────────────────────

fn run_cmd(cmd: &str, args: &[&str]) -> Result<()> {
    let status = Command::new(cmd).args(args).status().with_context(|| format!("Failed to run: {} {:?}", cmd, args))?;
    if !status.success() {
        anyhow::bail!("{} {:?} exited with {}", cmd, args, status);
    }
    Ok(())
}

// ── Existing proxy lifecycle (unchanged) ───────────────────────────

pub fn start_proxy() -> Result<()> {
    if systemd_available() {
        write_systemd_unit()?;
        run_cmd("systemctl", &["--user", "daemon-reload"])?;
        run_cmd("systemctl", &["--user", "enable", "--now", &format!("{}.service", SERVICE_NAME)])?;
        println!("Proxy started via systemd (user service)");
    } else {
        if let Some(pid) = read_proxy_pid().filter(|pid| process_matches_proxy(*pid)) {
            println!("Proxy already running (PID: {})", pid);
            return Ok(());
        }
        let _ = std::fs::remove_file(proxy_pid_path());
        let exe = std::env::current_exe()?;
        let mut child = Command::new(exe)
            .arg("proxy")
            .arg("serve")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .context("Failed to fork proxy process")?;
        if let Err(error) = write_pid_file(child.id()) {
            let _ = child.kill();
            return Err(error).context("Failed to persist proxy PID");
        }
        println!("Proxy started in background (PID: {})", child.id());
    }
    Ok(())
}

pub fn stop_proxy() -> Result<()> {
    if systemd_available() {
        run_cmd("systemctl", &["--user", "stop", &format!("{}.service", SERVICE_NAME)])?;
        println!("Proxy stopped (systemd)");
    } else {
        if let Some(pid) = read_proxy_pid() {
            if process_matches_proxy(pid) {
                terminate_process(pid)?;
                println!("Proxy stopped");
            } else {
                println!("No proxy process found");
            }
            let _ = std::fs::remove_file(proxy_pid_path());
        } else {
            println!("No proxy process found");
        }
    }
    Ok(())
}

pub fn proxy_status() -> Result<()> {
    if systemd_available() {
        let status = Command::new("systemctl").args(["--user", "status", &format!("{}.service", SERVICE_NAME)]).output()?;
        println!("{}", String::from_utf8_lossy(&status.stdout));
        if !status.status.success() {
            let stderr = String::from_utf8_lossy(&status.stderr);
            if !stderr.is_empty() {
                eprintln!("{}", stderr);
            }
        }
    } else {
        if let Some(pid) = read_proxy_pid().filter(|pid| process_matches_proxy(*pid)) {
            println!("Proxy running, PID: {}", pid);
        } else {
            println!("Proxy not running");
        }
    }
    Ok(())
}

fn write_systemd_unit() -> Result<PathBuf> {
    let dir = systemd_user_dir();
    std::fs::create_dir_all(&dir)?;
    let unit_path = dir.join(format!("{}.service", SERVICE_NAME));
    let exe = std::env::current_exe()?;
    let unit = format!(
        r#"[Unit]
Description=CCSwitch Proxy Server
After=network.target

[Service]
ExecStart={} proxy serve
Restart=on-failure
RestartSec=5

[Install]
WantedBy=default.target
"#,
        systemd_quote(&exe)
    );
    std::fs::write(&unit_path, unit)?;
    Ok(unit_path)
}

#[cfg(target_os = "linux")]
fn systemd_available() -> bool {
    Command::new("systemctl")
        .args(["--user", "show-environment"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

#[cfg(not(target_os = "linux"))]
fn systemd_available() -> bool {
    false
}

fn systemd_quote(path: &std::path::Path) -> String {
    let escaped = path.to_string_lossy().replace('\\', "\\\\").replace('"', "\\\"").replace('%', "%%");
    format!("\"{}\"", escaped)
}

fn proxy_pid_path() -> PathBuf {
    crate::core::config::data_dir().join("ccs-proxy.pid")
}

fn write_pid_file(pid: u32) -> Result<()> {
    let path = proxy_pid_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut options = std::fs::OpenOptions::new();
    options.create(true).truncate(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    }
    use std::io::Write;
    file.write_all(pid.to_string().as_bytes())?;
    file.sync_all()?;
    Ok(())
}

fn read_proxy_pid() -> Option<u32> {
    std::fs::read_to_string(proxy_pid_path()).ok()?.trim().parse().ok()
}

#[cfg(target_os = "linux")]
fn process_matches_proxy(pid: u32) -> bool {
    let same_executable = std::env::current_exe()
        .ok()
        .and_then(|path| std::fs::canonicalize(path).ok())
        .zip(std::fs::canonicalize(format!("/proc/{}/exe", pid)).ok())
        .is_some_and(|(current, running)| current == running);
    let command = std::fs::read(format!("/proc/{}/cmdline", pid)).ok();
    same_executable
        && command.is_some_and(|bytes| {
            let parts = bytes.split(|byte| *byte == 0).collect::<Vec<_>>();
            parts.windows(2).any(|pair| pair[0] == b"proxy" && pair[1] == b"serve")
        })
}

#[cfg(all(unix, not(target_os = "linux")))]
fn process_matches_proxy(pid: u32) -> bool {
    let current_exe = std::env::current_exe().ok().map(|path| path.to_string_lossy().to_string());
    current_exe.is_some_and(|current_exe| {
        Command::new("ps")
            .args(["-p", &pid.to_string(), "-o", "command="])
            .output()
            .ok()
            .filter(|output| output.status.success())
            .is_some_and(|output| {
                let command = String::from_utf8_lossy(&output.stdout);
                command.contains(&current_exe) && command.contains("proxy serve")
            })
    })
}

#[cfg(windows)]
fn process_matches_proxy(pid: u32) -> bool {
    let current_exe = std::env::current_exe().ok().map(|path| path.to_string_lossy().to_string());
    current_exe.is_some_and(|current_exe| {
        let command = format!("(Get-CimInstance Win32_Process -Filter \"ProcessId = {}\").CommandLine", pid);
        Command::new("powershell")
            .args(["-NoProfile", "-NonInteractive", "-Command", &command])
            .output()
            .ok()
            .filter(|output| output.status.success())
            .is_some_and(|output| {
                let command_line = String::from_utf8_lossy(&output.stdout);
                command_line.contains(&current_exe) && command_line.contains("proxy serve")
            })
    })
}

#[cfg(unix)]
fn terminate_process(pid: u32) -> Result<()> {
    run_cmd("kill", &[&pid.to_string()])
}

#[cfg(windows)]
fn terminate_process(pid: u32) -> Result<()> {
    run_cmd("taskkill", &["/PID", &pid.to_string(), "/T"])
}
