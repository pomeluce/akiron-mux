use std::sync::{Arc, Mutex};

use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use reqwest::Client;

use crate::core::config::ConfigManager;
use crate::core::env::resolve_api_key;

use super::metrics::record_metrics;
use super::transform;

/// Shared proxy state, held behind an Arc<Mutex<>> because `rusqlite::Connection`
/// uses internal `RefCell` and is therefore not `Sync`.
pub struct ProxyState {
    pub mgr: Arc<Mutex<ConfigManager>>,
    pub client: Client,
}

struct UpstreamInfo {
    api_url: String,
    auth_token: String,
    opus_model: String,
    sonnet_model: String,
    haiku_model: String,
    subagent_model: String,
}

/// Handle all incoming Anthropic-compatible API requests.
///
/// Reads the active provider/profile from SQLite, replaces the Authorization
/// header, transforms the model name in both request and response bodies,
/// and streams the response back.
pub async fn proxy_handler(
    State(state): State<Arc<ProxyState>>,
    req: Request<Body>,
) -> Response {
    // ── Extract request data we need BEFORE consuming the body ──────
    let method = req.method().clone();
    let original_headers = req.headers().clone();
    let path = req
        .uri()
        .path_and_query()
        .map(|pq| pq.as_str())
        .unwrap_or("/")
        .to_string();

    tracing::info!("Proxy: {} {}", method, path);

    // Resolve the active upstream target, auth token, and model mapping
    let upstream = {
        let mgr = match state.mgr.lock() {
            Ok(g) => g,
            Err(e) => {
                tracing::error!("Mutex poisoned: {}", e);
                return (StatusCode::INTERNAL_SERVER_ERROR, "Internal error".to_string())
                    .into_response();
            }
        };
        match get_active_upstream(&mgr) {
            Ok(v) => v,
            Err(e) => {
                tracing::error!("Failed to resolve upstream: {}", e);
                return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response();
            }
        }
    };

    // Build upstream URL preserving path + query
    let upstream_url = format!("{}{}", upstream.api_url.trim_end_matches('/'), path);

    // Read entire request body
    let body_bytes = match axum::body::to_bytes(req.into_body(), 64 * 1024 * 1024).await {
        Ok(b) => b,
        Err(e) => {
            tracing::error!("Failed to read request body: {}", e);
            return (StatusCode::BAD_REQUEST, "Failed to read request body").into_response();
        }
    };

    // ── Transform request body: replace Claude model → actual upstream model ──
    let (transformed_body, original_model, actual_model) = match transform::transform_request_body(
        &body_bytes,
        &upstream.opus_model,
        &upstream.sonnet_model,
        &upstream.haiku_model,
        &upstream.subagent_model,
    ) {
        Ok(v) => v,
        Err(e) => {
            // If we can't parse the body, forward it as-is (e.g. non-JSON health checks)
            tracing::debug!("Body transform skipped: {}", e);
            (body_bytes.to_vec(), String::new(), String::new())
        }
    };

    let is_v1_messages = path.starts_with("/v1/messages");
    if is_v1_messages && !original_model.is_empty() {
        tracing::info!(
            "Model transform: original={} → actual={}",
            original_model,
            actual_model,
        );
    }

    // Build upstream request
    let headers = prepare_upstream_headers(&original_headers, &upstream.auth_token);
    let body_len = transformed_body.len();
    tracing::info!(
        "Upstream request: {} {} body_len={} auth_set={}",
        method,
        upstream_url,
        body_len,
        upstream.auth_token.len() > 0,
    );

    let upstream_req = state
        .client
        .request(method, &upstream_url)
        .headers(headers)
        .body(reqwest::Body::from(transformed_body))
        .build();

    let upstream_req = match upstream_req {
        Ok(r) => r,
        Err(e) => {
            tracing::error!("Failed to build upstream request: {}", e);
            return (StatusCode::INTERNAL_SERVER_ERROR, "Failed to build upstream request")
                .into_response();
        }
    };

    // Execute the upstream request
    match state.client.execute(upstream_req).await {
        Ok(resp) => {
            let status = resp.status();
            let response_headers = resp.headers().clone();
            let content_type = response_headers
                .get("content-type")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("(none)");
            tracing::info!(
                "Upstream response: status={} content-type={} upstream_url={}",
                status,
                content_type,
                upstream_url,
            );

            if !status.is_success() {
                let body_text = resp.text().await.unwrap_or_else(|_| "(unreadable)".into());
                tracing::error!(
                    "Upstream error: status={} body={}",
                    status,
                    &body_text[..body_text.len().min(500)],
                );
                return (status, body_text).into_response();
            }

            // Record metrics from response headers (best-effort, non-blocking)
            match state.mgr.lock() {
                Ok(mgr) => {
                    if let Err(e) = record_metrics(&mgr, &status, &response_headers) {
                        tracing::warn!("Failed to record metrics: {}", e);
                    }
                }
                Err(e) => {
                    tracing::warn!("Mutex poisoned during metrics recording: {}", e);
                }
            }

            // ── Transform SSE response stream ──
            let body = if is_v1_messages && !original_model.is_empty() {
                transform::transform_response_stream(
                    resp.bytes_stream(),
                    original_model,
                    actual_model,
                )
            } else {
                Body::from_stream(resp.bytes_stream())
            };

            let mut response = Response::new(body);
            *response.status_mut() = status;
            *response.headers_mut() = response_headers;
            response
        }
        Err(e) => {
            tracing::error!("Upstream request failed: {} upstream_url={}", e, upstream_url);
            (StatusCode::BAD_GATEWAY, format!("Upstream error: {}", e)).into_response()
        }
    }
}

/// Look up the active provider and profile from the database and return
/// upstream connection info including model mapping.
fn get_active_upstream(mgr: &ConfigManager) -> anyhow::Result<UpstreamInfo> {
    let provider_id = mgr
        .db()
        .get_setting("active_provider")
        .ok_or_else(|| anyhow::anyhow!("No active provider set"))?;
    let profile_id = mgr
        .db()
        .get_setting("active_profile")
        .ok_or_else(|| anyhow::anyhow!("No active profile set"))?;

    let (provider, profile) = mgr
        .find_profile(&provider_id, &profile_id)?
        .ok_or_else(|| anyhow::anyhow!("Profile {}/{} not found", provider_id, profile_id))?;

    let token = resolve_api_key(&provider.api_key);
    Ok(UpstreamInfo {
        api_url: provider.api_url,
        auth_token: token,
        opus_model: profile.opus.clone(),
        sonnet_model: profile.sonnet.clone(),
        haiku_model: profile.haiku.clone(),
        subagent_model: profile.subagent.clone(),
    })
}

/// Clone the original headers, replace Authorization with the real upstream
/// token, and strip hop-by-hop / client-side headers that must not be forwarded.
fn prepare_upstream_headers(original: &HeaderMap, new_token: &str) -> HeaderMap {
    let mut headers = original.clone();
    // Replace auth header with real upstream API key
    if let Ok(hv) = HeaderValue::from_str(&format!("Bearer {}", new_token)) {
        headers.insert("Authorization", hv);
    }
    // Strip hop-by-hop / body-dependent headers — reqwest will set the correct values
    headers.remove("host");
    headers.remove("Host");
    headers.remove("connection");
    headers.remove("Connection");
    headers.remove("transfer-encoding");
    headers.remove("Transfer-Encoding");
    headers.remove("content-length");
    headers.remove("Content-Length");
    headers
}
