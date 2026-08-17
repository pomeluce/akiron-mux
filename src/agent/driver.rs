use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AgentKind {
    Claude,
    Codex,
}

impl AgentKind {
    pub fn display_name(self) -> &'static str {
        match self {
            Self::Claude => "Claude Code",
            Self::Codex => "Codex",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LaunchMode {
    New,
    ResumePicker,
    Resume { native_session_id: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchRequest {
    pub agent: AgentKind,
    pub cwd: PathBuf,
    pub mode: LaunchMode,
    pub managed_session_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchSpec {
    pub program: String,
    pub args: Vec<String>,
    pub cwd: PathBuf,
    pub env: Vec<(String, String)>,
}

pub(crate) trait AgentDriver {
    fn launch_spec(&self, request: &LaunchRequest) -> anyhow::Result<LaunchSpec>;
}

pub fn launch_spec(request: &LaunchRequest) -> anyhow::Result<LaunchSpec> {
    match request.agent {
        AgentKind::Claude => super::claude::ClaudeDriver.launch_spec(request),
        AgentKind::Codex => super::codex::CodexDriver.launch_spec(request),
    }
}

pub(super) fn session_event_program() -> PathBuf {
    std::env::current_exe().unwrap_or_else(|_| PathBuf::from(if cfg!(windows) { "akmux-sessiond.exe" } else { "akmux-sessiond" }))
}

pub(super) fn session_event_command(managed_session_id: &str, event: &str) -> String {
    let program = session_event_program().to_string_lossy().into_owned();
    if cfg!(windows) {
        format!("\"{}\" session-event {} {}", program.replace('"', "\"\""), managed_session_id, event)
    } else {
        format!("{} session-event {} {}", shell_quote(&program), managed_session_id, event)
    }
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

#[cfg(test)]
mod tests {
    use super::{launch_spec, AgentKind, LaunchMode, LaunchRequest};
    use std::path::PathBuf;

    #[test]
    fn builds_claude_resume_command() {
        let spec = launch_spec(&LaunchRequest {
            agent: AgentKind::Claude,
            cwd: PathBuf::from("/tmp/project"),
            mode: LaunchMode::Resume {
                native_session_id: "claude-session".into(),
            },
            managed_session_id: None,
        })
        .unwrap();

        assert_eq!(spec.program, "claude");
        assert_eq!(spec.args, ["--resume", "claude-session"]);
    }

    #[test]
    fn builds_native_resume_picker_commands() {
        let claude = launch_spec(&LaunchRequest {
            agent: AgentKind::Claude,
            cwd: PathBuf::from("/tmp/project"),
            mode: LaunchMode::ResumePicker,
            managed_session_id: None,
        })
        .unwrap();
        let codex = launch_spec(&LaunchRequest {
            agent: AgentKind::Codex,
            cwd: PathBuf::from("/tmp/project"),
            mode: LaunchMode::ResumePicker,
            managed_session_id: None,
        })
        .unwrap();

        assert_eq!(claude.args, ["--resume"]);
        assert_eq!(codex.args, ["resume"]);
    }

    #[test]
    fn builds_codex_resume_command() {
        let spec = launch_spec(&LaunchRequest {
            agent: AgentKind::Codex,
            cwd: PathBuf::from("/tmp/project"),
            mode: LaunchMode::Resume {
                native_session_id: "codex-session".into(),
            },
            managed_session_id: None,
        })
        .unwrap();

        assert_eq!(spec.program, "codex");
        assert_eq!(spec.args, ["resume", "codex-session"]);
    }
}
