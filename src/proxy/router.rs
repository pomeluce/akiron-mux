use std::sync::{Arc, Mutex};

use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use bytes::{Bytes, BytesMut};
use reqwest::Client;
use tokio_stream::StreamExt;

use crate::core::config::ConfigManager;
use crate::core::env::resolve_api_key;
use crate::core::models::validate_provider;

use super::transform;

const MAX_UPSTREAM_ERROR_BODY: usize = 1024 * 1024;

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
pub async fn proxy_handler(State(state): State<Arc<ProxyState>>, req: Request<Body>) -> Response {
    // ── Extract request data we need BEFORE consuming the body ──────
    let method = req.method().clone();
    let original_headers = req.headers().clone();
    let request_path = req.uri().path().to_string();
    let path_and_query = req.uri().path_and_query().map(|pq| pq.as_str()).unwrap_or("/").to_string();

    tracing::info!("Proxy: {} {}", method, request_path);

    // Resolve the active upstream target, auth token, and model mapping
    let upstream = {
        let mgr = match state.mgr.lock() {
            Ok(g) => g,
            Err(e) => {
                tracing::error!("Mutex poisoned: {}", e);
                return (StatusCode::INTERNAL_SERVER_ERROR, "Internal error".to_string()).into_response();
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
    let upstream_url = format!("{}{}", upstream.api_url.trim_end_matches('/'), path_and_query);
    let upstream_log_url = format!("{}{}", upstream.api_url.trim_end_matches('/'), request_path);

    // Read entire request body
    let body_bytes = match axum::body::to_bytes(req.into_body(), 64 * 1024 * 1024).await {
        Ok(b) => b,
        Err(e) => {
            tracing::error!("Failed to read request body: {}", e);
            return (StatusCode::BAD_REQUEST, "Failed to read request body").into_response();
        }
    };

    let is_v1_messages = request_path == "/v1/messages";
    // ── Transform request body: replace Claude model → actual upstream model ──
    let (transformed_body, original_model, actual_model) =
        match transform::transform_request_body(&body_bytes, &upstream.opus_model, &upstream.sonnet_model, &upstream.haiku_model, &upstream.subagent_model) {
            Ok(v) => v,
            Err(e) => {
                if is_v1_messages {
                    tracing::warn!("Invalid /v1/messages request body: {}", e);
                    return (StatusCode::BAD_REQUEST, "Invalid messages request body").into_response();
                }
                tracing::debug!("Body transform skipped for non-message request: {}", e);
                (body_bytes.to_vec(), String::new(), String::new())
            }
        };

    if is_v1_messages && !original_model.is_empty() {
        tracing::info!("Model transform: original={} → actual={}", original_model, actual_model,);
    }

    // Build upstream request
    let headers = match prepare_upstream_headers(&original_headers, &upstream.auth_token) {
        Ok(headers) => headers,
        Err(error) => {
            tracing::error!("Invalid upstream authentication header: {}", error);
            return (StatusCode::INTERNAL_SERVER_ERROR, "Invalid upstream authentication configuration").into_response();
        }
    };
    let body_len = transformed_body.len();
    tracing::info!(
        "Upstream request: {} {} body_len={} auth_set={}",
        method,
        upstream_log_url,
        body_len,
        !upstream.auth_token.is_empty(),
    );

    let upstream_req = state
        .client
        .request(method, &upstream_url)
        .headers(headers)
        .body(reqwest::Body::from(transformed_body))
        .build();

    let upstream_req = match upstream_req {
        Ok(r) => r,
        Err(_) => {
            tracing::error!("Failed to build upstream request for {}", request_path);
            return (StatusCode::INTERNAL_SERVER_ERROR, "Failed to build upstream request").into_response();
        }
    };

    // Execute the upstream request
    match state.client.execute(upstream_req).await {
        Ok(resp) => {
            let status = resp.status();
            let response_headers = resp.headers().clone();
            let content_type = response_headers.get("content-type").and_then(|v| v.to_str().ok()).unwrap_or("(none)");
            tracing::info!("Upstream response: status={} content-type={} upstream_url={}", status, content_type, upstream_log_url,);

            if !status.is_success() {
                let (body, truncated) = read_limited_response_body(resp, MAX_UPSTREAM_ERROR_BODY)
                    .await
                    .unwrap_or_else(|_| (Bytes::from_static(b"(unreadable)"), false));
                tracing::error!("Upstream error: status={} body_len={} truncated={}", status, body.len(), truncated,);
                let mut response = Response::new(Body::from(body));
                *response.status_mut() = status;
                *response.headers_mut() = sanitize_response_headers(response_headers);
                return response;
            }

            // ── Transform SSE response stream ──
            let body = if is_v1_messages && !original_model.is_empty() {
                transform::transform_response_stream(resp.bytes_stream(), original_model, actual_model)
            } else {
                Body::from_stream(resp.bytes_stream())
            };

            let mut response = Response::new(body);
            *response.status_mut() = status;
            *response.headers_mut() = sanitize_response_headers(response_headers);
            response
        }
        Err(error) => {
            tracing::error!(
                "Upstream request failed for {} (timeout={}, connect={})",
                upstream_log_url,
                error.is_timeout(),
                error.is_connect(),
            );
            (StatusCode::BAD_GATEWAY, "Upstream request failed").into_response()
        }
    }
}

async fn read_limited_response_body(response: reqwest::Response, limit: usize) -> Result<(Bytes, bool), reqwest::Error> {
    let mut stream = response.bytes_stream();
    let mut body = BytesMut::new();
    let mut truncated = false;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        let remaining = limit.saturating_sub(body.len());
        if chunk.len() > remaining {
            body.extend_from_slice(&chunk[..remaining]);
            truncated = true;
            break;
        }
        body.extend_from_slice(&chunk);
    }
    Ok((body.freeze(), truncated))
}

/// Look up the active provider and profile from the database and return
/// upstream connection info including model mapping.
fn get_active_upstream(mgr: &ConfigManager) -> anyhow::Result<UpstreamInfo> {
    let provider_id = mgr.db().get_setting("active_provider").ok_or_else(|| anyhow::anyhow!("No active provider set"))?;
    let profile_id = mgr.db().get_setting("active_profile").ok_or_else(|| anyhow::anyhow!("No active profile set"))?;

    let (provider, profile) = mgr
        .find_profile(&provider_id, &profile_id)?
        .ok_or_else(|| anyhow::anyhow!("Profile {}/{} not found", provider_id, profile_id))?;

    let token = resolve_api_key(&provider.api_key);
    if token.is_empty() {
        anyhow::bail!("API key unavailable for provider '{}'", provider.id);
    }
    validate_provider(&provider)?;
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
fn prepare_upstream_headers(original: &HeaderMap, new_token: &str) -> anyhow::Result<HeaderMap> {
    let mut headers = original.clone();
    let connection_tokens = headers
        .get("connection")
        .and_then(|value| value.to_str().ok())
        .map(|value| value.split(',').map(|token| token.trim().to_ascii_lowercase()).collect::<Vec<_>>())
        .unwrap_or_default();
    for name in [
        "authorization",
        "x-api-key",
        "host",
        "connection",
        "keep-alive",
        "proxy-authenticate",
        "proxy-authorization",
        "te",
        "trailer",
        "transfer-encoding",
        "upgrade",
        "content-length",
    ] {
        headers.remove(name);
    }
    for name in connection_tokens {
        headers.remove(name);
    }
    let bearer = HeaderValue::from_str(&format!("Bearer {}", new_token))?;
    let api_key = HeaderValue::from_str(new_token)?;
    headers.insert("authorization", bearer);
    headers.insert("x-api-key", api_key);
    Ok(headers)
}

fn sanitize_response_headers(mut headers: HeaderMap) -> HeaderMap {
    let connection_tokens = headers
        .get("connection")
        .and_then(|value| value.to_str().ok())
        .map(|value| value.split(',').map(|token| token.trim().to_ascii_lowercase()).collect::<Vec<_>>())
        .unwrap_or_default();
    for name in [
        "connection",
        "keep-alive",
        "proxy-authenticate",
        "proxy-authorization",
        "te",
        "trailer",
        "transfer-encoding",
        "upgrade",
        "content-length",
    ] {
        headers.remove(name);
    }
    for name in connection_tokens {
        headers.remove(name);
    }
    headers
}

#[cfg(test)]
mod tests {
    use super::prepare_upstream_headers;
    use axum::http::{HeaderMap, HeaderValue};

    #[test]
    fn replaces_auth_and_removes_hop_by_hop_headers() {
        let mut input = HeaderMap::new();
        input.insert("authorization", HeaderValue::from_static("Bearer dummy"));
        input.insert("x-api-key", HeaderValue::from_static("dummy"));
        input.insert("connection", HeaderValue::from_static("keep-alive, x-remove-me"));
        input.insert("x-remove-me", HeaderValue::from_static("value"));
        input.insert("content-length", HeaderValue::from_static("10"));
        let headers = prepare_upstream_headers(&input, "real-key").unwrap();
        assert_eq!(headers["authorization"], "Bearer real-key");
        assert_eq!(headers["x-api-key"], "real-key");
        assert!(!headers.contains_key("connection"));
        assert!(!headers.contains_key("x-remove-me"));
        assert!(!headers.contains_key("content-length"));
    }
}
