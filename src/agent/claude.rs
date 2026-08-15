use super::driver::{AgentDriver, LaunchMode, LaunchRequest, LaunchSpec};

pub(crate) struct ClaudeDriver;

impl AgentDriver for ClaudeDriver {
    fn launch_spec(&self, request: &LaunchRequest) -> anyhow::Result<LaunchSpec> {
        let args = match &request.mode {
            LaunchMode::New => Vec::new(),
            LaunchMode::ResumePicker => vec!["--resume".into()],
            LaunchMode::Resume { native_session_id } => vec!["--resume".into(), native_session_id.clone()],
        };
        Ok(LaunchSpec {
            program: "claude".into(),
            args,
            cwd: request.cwd.clone(),
            env: Vec::new(),
        })
    }
}
