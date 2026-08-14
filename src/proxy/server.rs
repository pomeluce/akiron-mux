use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use axum::Router;
use tokio::net::TcpListener;

use crate::core::config::ConfigManager;

use super::router::{proxy_handler, ProxyState};

/// An HTTP proxy server that intercepts Anthropic-compatible API calls
/// and forwards them to the currently active upstream provider.
pub struct ProxyServer {
    mgr: Arc<Mutex<ConfigManager>>,
}

impl ProxyServer {
    /// Create a new proxy server backed by the given config manager.
    pub fn new(mgr: ConfigManager) -> Self {
        ProxyServer { mgr: Arc::new(Mutex::new(mgr)) }
    }

    /// Run the proxy server in the foreground, blocking until shutdown.
    ///
    /// Listens on `127.0.0.1:<port>` and handles all requests by forwarding
    /// them to the active upstream provider.
    pub async fn serve(self, port: u16) -> anyhow::Result<()> {
        let client = reqwest::Client::builder()
            .connect_timeout(std::time::Duration::from_secs(30))
            .timeout(std::time::Duration::from_secs(600))
            .build()?;

        let state = Arc::new(ProxyState { mgr: self.mgr.clone(), client });

        let app = Router::<Arc<ProxyState>>::new().fallback(proxy_handler).with_state(state);

        let addr = SocketAddr::from(([127, 0, 0, 1], port));
        let listener = TcpListener::bind(addr).await?;
        tracing::info!("Proxy listening on http://{}", addr);

        axum::serve(listener, app)
            .with_graceful_shutdown(async {
                tokio::signal::ctrl_c().await.ok();
                tracing::info!("Shutting down proxy gracefully...");
            })
            .await?;
        Ok(())
    }
}
