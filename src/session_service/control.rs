use std::{
    net::{IpAddr, Ipv4Addr, SocketAddr, TcpStream},
    path::PathBuf,
    process::{Command, Stdio},
    time::Duration,
};

use anyhow::{Context, Result};

use crate::{core::config::ConfigManager, db::Db};

pub const ENABLED_SETTING: &str = "session_service_enabled";
const SERVICE_NAME: &str = "akmux-sessiond";
const DEFAULT_PORT: u16 = 17321;

pub fn configured_enabled() -> Result<bool> {
    let db = Db::open(&crate::core::config::db_path())?;
    Ok(db.get_setting(ENABLED_SETTING).is_some_and(|value| value == "true"))
}

pub fn enabled(mgr: &ConfigManager) -> bool {
    mgr.get_setting(ENABLED_SETTING).is_some_and(|value| value == "true")
}

pub fn set_enabled(mgr: &ConfigManager, value: bool) -> Result<()> {
    mgr.set_setting(ENABLED_SETTING, &value.to_string())?;
    if value {
        if let Err(error) = start() {
            let _ = mgr.set_setting(ENABLED_SETTING, "false");
            return Err(error);
        }
    } else {
        stop()?;
    }
    Ok(())
}

pub fn reconcile(mgr: &ConfigManager) -> Result<()> {
    if enabled(mgr) {
        start()
    } else {
        stop()
    }
}

pub fn is_running() -> bool {
    TcpStream::connect_timeout(&SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), configured_port()), Duration::from_millis(150)).is_ok()
}

fn start() -> Result<()> {
    if is_running() {
        return Ok(());
    }

    #[cfg(target_os = "linux")]
    if systemd_available() && systemd_unit_exists() {
        let status = Command::new("systemctl")
            .args(["--user", "start", &format!("{SERVICE_NAME}.service")])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .context("Failed to start the AkironMux session service with systemd")?;
        if status.success() {
            return Ok(());
        }
    }

    let executable = daemon_executable();
    let mut command = Command::new(&executable);
    command.stdout(Stdio::null()).stderr(Stdio::null()).stdin(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        const DETACHED_PROCESS: u32 = 0x0000_0008;
        command.creation_flags(CREATE_NEW_PROCESS_GROUP | DETACHED_PROCESS);
    }
    command.spawn().with_context(|| format!("Failed to start {}", executable.display()))?;
    Ok(())
}

fn stop() -> Result<()> {
    #[cfg(target_os = "linux")]
    if systemd_available() && systemd_unit_exists() {
        let _ = Command::new("systemctl")
            .args(["--user", "stop", &format!("{SERVICE_NAME}.service")])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }

    if let Some(pid) = service_pid().filter(|pid| process_matches_service(*pid)) {
        terminate_process(pid)?;
    }
    let _ = std::fs::remove_file(service_state_path());
    Ok(())
}

fn configured_port() -> u16 {
    std::env::var("AKMUX_SESSION_PORT")
        .or_else(|_| std::env::var("CCSWITCH_SESSION_PORT"))
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(DEFAULT_PORT)
}

fn daemon_executable() -> PathBuf {
    if let Some(path) = std::env::var_os("AKMUX_SESSIOND_PATH") {
        return PathBuf::from(path);
    }
    let name = if cfg!(windows) { "akmux-sessiond.exe" } else { "akmux-sessiond" };
    std::env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(|parent| parent.join(name)))
        .filter(|path| path.is_file())
        .unwrap_or_else(|| PathBuf::from(name))
}

fn service_state_path() -> PathBuf {
    crate::core::config::data_dir().join("session-service.json")
}

fn service_pid() -> Option<u32> {
    let value: serde_json::Value = serde_json::from_slice(&std::fs::read(service_state_path()).ok()?).ok()?;
    value.get("pid")?.as_u64()?.try_into().ok()
}

#[cfg(target_os = "linux")]
fn systemd_available() -> bool {
    Command::new("systemctl")
        .args(["--user", "show-environment"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

#[cfg(target_os = "linux")]
fn systemd_unit_exists() -> bool {
    Command::new("systemctl")
        .args(["--user", "cat", &format!("{SERVICE_NAME}.service")])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

#[cfg(target_os = "linux")]
fn process_matches_service(pid: u32) -> bool {
    std::fs::read(format!("/proc/{pid}/cmdline"))
        .ok()
        .is_some_and(|command| command.split(|byte| *byte == 0).any(|part| part.ends_with(b"akmux-sessiond")))
}

#[cfg(all(unix, not(target_os = "linux")))]
fn process_matches_service(pid: u32) -> bool {
    Command::new("ps")
        .args(["-p", &pid.to_string(), "-o", "command="])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .is_some_and(|output| String::from_utf8_lossy(&output.stdout).contains("akmux-sessiond"))
}

#[cfg(windows)]
fn process_matches_service(pid: u32) -> bool {
    let command = format!("(Get-CimInstance Win32_Process -Filter \"ProcessId = {pid}\").CommandLine");
    Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", &command])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .is_some_and(|output| String::from_utf8_lossy(&output.stdout).contains("akmux-sessiond"))
}

#[cfg(unix)]
fn terminate_process(pid: u32) -> Result<()> {
    let status = Command::new("kill").arg(pid.to_string()).status()?;
    anyhow::ensure!(status.success(), "Failed to stop AkironMux session service process {pid}");
    Ok(())
}

#[cfg(windows)]
fn terminate_process(pid: u32) -> Result<()> {
    let status = Command::new("taskkill").args(["/PID", &pid.to_string(), "/T", "/F"]).status()?;
    anyhow::ensure!(status.success(), "Failed to stop AkironMux session service process {pid}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backend_setting_defaults_to_disabled_and_persists() {
        let directory = tempfile::tempdir().unwrap();
        let defaults = directory.path().join("missing-defaults.toml");
        let manager = ConfigManager::new(&directory.path().join("akmux.db"), Some(&defaults)).unwrap();

        assert!(!enabled(&manager));
        manager.set_setting(ENABLED_SETTING, "true").unwrap();
        assert!(enabled(&manager));
    }
}
