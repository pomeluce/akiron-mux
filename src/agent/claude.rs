use super::driver::{session_event_command, AgentDriver, LaunchMode, LaunchRequest, LaunchSpec};

pub(crate) struct ClaudeDriver;

impl AgentDriver for ClaudeDriver {
    fn launch_spec(&self, request: &LaunchRequest) -> anyhow::Result<LaunchSpec> {
        let mut args = match &request.mode {
            LaunchMode::New => Vec::new(),
            LaunchMode::ResumePicker => vec!["--resume".into()],
            LaunchMode::Resume { native_session_id } => vec!["--resume".into(), native_session_id.clone()],
        };
        if let Some(managed_session_id) = request.managed_session_id.as_deref() {
            let permission = session_event_command(managed_session_id, "input");
            let completed = session_event_command(managed_session_id, "claude-completed");
            let settings = serde_json::json!({
                "hooks": {
                    "PermissionRequest": [{ "hooks": [{ "type": "command", "command": permission }] }],
                    "Stop": [{ "hooks": [{ "type": "command", "command": completed }] }]
                }
            });
            args.push("--settings".into());
            args.push(settings.to_string());
        }
        Ok(LaunchSpec {
            program: "claude".into(),
            args,
            cwd: request.cwd.clone(),
            env: Vec::new(),
        })
    }
}
