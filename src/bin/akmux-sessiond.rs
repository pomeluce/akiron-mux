#[tokio::main]
async fn main() {
    let mut arguments = std::env::args().skip(1);
    if arguments.next().as_deref() == Some("session-event") {
        let Some(managed_session_id) = arguments.next() else {
            return;
        };
        let Some(event) = arguments.next() else {
            return;
        };
        let payload = arguments.next().or_else(|| {
            if !matches!(event.as_str(), "completed" | "claude-completed") {
                return None;
            }
            let mut payload = String::new();
            std::io::Read::read_to_string(&mut std::io::stdin(), &mut payload).ok()?;
            (!payload.trim().is_empty()).then_some(payload)
        });
        let _ = ccswitch::session_service::emit_session_event(&managed_session_id, &event, payload.as_deref()).await;
        return;
    }
    tracing_subscriber::fmt().with_target(false).init();
    if let Err(error) = ccswitch::session_service::run_from_env().await {
        tracing::error!("Session service failed: {error:#}");
        std::process::exit(1);
    }
}
