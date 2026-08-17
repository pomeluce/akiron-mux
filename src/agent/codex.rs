use super::driver::{session_event_program, AgentDriver, LaunchMode, LaunchRequest, LaunchSpec};

pub(crate) struct CodexDriver;

impl AgentDriver for CodexDriver {
    fn launch_spec(&self, request: &LaunchRequest) -> anyhow::Result<LaunchSpec> {
        let mut args = Vec::new();
        if let Some(managed_session_id) = request.managed_session_id.as_deref() {
            let notify = toml::Value::Array(vec![
                toml::Value::String(session_event_program().to_string_lossy().into_owned()),
                toml::Value::String("session-event".into()),
                toml::Value::String(managed_session_id.into()),
                toml::Value::String("codex-completed".into()),
            ]);
            args.extend([
                "-c".into(),
                format!("notify={notify}"),
                "-c".into(),
                "tui.notifications=[\"approval-requested\"]".into(),
                "-c".into(),
                "tui.notification_method=\"osc9\"".into(),
                "-c".into(),
                "tui.notification_condition=\"always\"".into(),
            ]);
        }
        args.extend(match &request.mode {
            LaunchMode::New => Vec::new(),
            LaunchMode::ResumePicker => vec!["resume".into()],
            LaunchMode::Resume { native_session_id } => vec!["resume".into(), native_session_id.clone()],
        });
        Ok(LaunchSpec {
            program: "codex".into(),
            args,
            cwd: request.cwd.clone(),
            env: Vec::new(),
        })
    }
}
