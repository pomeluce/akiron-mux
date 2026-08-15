#[tokio::main]
async fn main() {
    tracing_subscriber::fmt().with_target(false).init();
    if let Err(error) = ccswitch::session_service::run_from_env().await {
        tracing::error!("Session service failed: {error:#}");
        std::process::exit(1);
    }
}
