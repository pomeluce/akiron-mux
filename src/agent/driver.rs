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
        })
        .unwrap();
        let codex = launch_spec(&LaunchRequest {
            agent: AgentKind::Codex,
            cwd: PathBuf::from("/tmp/project"),
            mode: LaunchMode::ResumePicker,
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
        })
        .unwrap();

        assert_eq!(spec.program, "codex");
        assert_eq!(spec.args, ["resume", "codex-session"]);
    }
}
