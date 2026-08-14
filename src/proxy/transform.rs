use bytes::Bytes;
use serde_json::Value;
use tokio::sync::mpsc;
use tokio_stream::{wrappers::ReceiverStream, StreamExt};

const MAX_SSE_EVENT_SIZE: usize = 8 * 1024 * 1024;

/// Transform the request body: replace `model` with the actual upstream model,
/// return the original model name so it can be restored in the response.
pub fn transform_request_body(body_bytes: &[u8], opus_model: &str, sonnet_model: &str, haiku_model: &str, subagent_model: &str) -> Result<(Vec<u8>, String, String), String> {
    let mut json: Value = serde_json::from_slice(body_bytes).map_err(|e| format!("JSON parse: {}", e))?;

    let original_model = json["model"].as_str().unwrap_or("").to_string();

    if original_model.is_empty() {
        return Err("No model field in request body".into());
    }

    // Determine which model to use:
    // Map each Claude Code model class to its configured upstream model.
    let lower = original_model.to_lowercase();
    let actual_model = if lower.contains("ccswitch-subagent") {
        subagent_model
    } else if lower.contains("opus") {
        opus_model
    } else if lower.contains("sonnet") {
        sonnet_model
    } else if lower.contains("haiku") {
        haiku_model
    } else {
        sonnet_model
    };

    // Strip [1m] suffix for the actual API request (CCSwitch convention)
    let api_model = actual_model.replace("[1m]", "");
    json["model"] = Value::String(api_model);

    let new_body = serde_json::to_vec(&json).map_err(|e| format!("JSON serialize: {}", e))?;

    Ok((new_body, original_model, actual_model.to_string()))
}

/// Transform a single SSE event's `data:` JSON payload:
/// - Replace `message.model` → original_model
/// - Add `ccs_model` → actual_model
/// - Add `ccs_proxy` → true
fn transform_sse_data(data_json: &str, original_model: &str, actual_model: &str) -> Result<String, String> {
    let trimmed = data_json.trim();
    if trimmed.is_empty() || !trimmed.starts_with('{') {
        return Ok(data_json.to_string());
    }

    let mut json: Value = serde_json::from_str(trimmed).map_err(|e| format!("SSE JSON parse: {}: {}", e, trimmed))?;

    // Replace message.model + inject ccs_ fields inside message
    let ccs_model = actual_model.replace("[1m]", "");
    if let Some(msg) = json.get_mut("message") {
        if let Some(model) = msg.get_mut("model") {
            // If the actual upstream model had [1m] → also tag the original
            // so Claude Code recognizes the 1M context variant when resuming
            let restored = if actual_model.contains("[1m]") {
                format!("{}[1m]", original_model)
            } else {
                original_model.to_string()
            };
            *model = Value::String(restored);
        }
        // Put ccs_ fields inside message so Claude Code preserves them in JSONL
        if let Some(msg_obj) = msg.as_object_mut() {
            msg_obj.insert("ccs_model".into(), Value::String(ccs_model));
            msg_obj.insert("ccs_proxy".into(), Value::Bool(true));
        }
    }

    let result = serde_json::to_string(&json).map_err(|e| format!("SSE JSON serialize: {}", e))?;
    Ok(result)
}

/// Process raw SSE bytes: find `data:` lines, transform the JSON, return modified bytes.
fn transform_sse_chunk(bytes: &[u8], original_model: &str, actual_model: &str) -> Vec<u8> {
    let text = String::from_utf8_lossy(bytes);
    let mut result = String::with_capacity(text.len() + 128);
    let mut data_start = 0usize;

    for (i, _) in text.match_indices('\n') {
        let line = &text[data_start..i];
        if let Some(payload) = line.strip_prefix("data: ") {
            match transform_sse_data(payload, original_model, actual_model) {
                Ok(transformed) => {
                    result.push_str("data: ");
                    result.push_str(&transformed);
                }
                Err(_) => {
                    result.push_str(line);
                }
            }
        } else {
            result.push_str(line);
        }
        result.push('\n');
        data_start = i + 1;
    }

    if data_start < text.len() {
        let line = &text[data_start..];
        if let Some(payload) = line.strip_prefix("data: ") {
            match transform_sse_data(payload, original_model, actual_model) {
                Ok(transformed) => {
                    result.push_str("data: ");
                    result.push_str(&transformed);
                }
                Err(_) => result.push_str(line),
            }
        } else {
            result.push_str(line);
        }
    }

    result.into_bytes()
}

/// Transform an upstream SSE byte stream: split on \n\n → transform data lines → re-emit.
/// Returns an axum Body for the proxy response.
pub fn transform_response_stream(
    upstream_body: impl futures_core::Stream<Item = Result<Bytes, reqwest::Error>> + Send + 'static,
    original_model: String,
    actual_model: String,
) -> axum::body::Body {
    let (tx, rx) = mpsc::channel::<Result<Bytes, axum::Error>>(32);
    let mut stream = Box::pin(upstream_body);

    tokio::spawn(async move {
        let mut buffer = Vec::new();
        loop {
            match stream.as_mut().next().await {
                Some(Ok(bytes)) => {
                    buffer.extend_from_slice(&bytes);
                    // Emit complete SSE events (separated by \n\n)
                    while let Some(end) = find_sse_event_end(&buffer) {
                        let event_bytes: Vec<u8> = buffer.drain(..end).collect();
                        let transformed = transform_sse_chunk(&event_bytes, &original_model, &actual_model);
                        if tx.send(Ok(Bytes::from(transformed))).await.is_err() {
                            return; // receiver dropped
                        }
                    }
                    if buffer.len() > MAX_SSE_EVENT_SIZE {
                        let error = std::io::Error::new(std::io::ErrorKind::InvalidData, "upstream SSE event exceeded 8 MiB");
                        let _ = tx.send(Err(axum::Error::new(error))).await;
                        return;
                    }
                }
                Some(Err(e)) => {
                    let _ = tx.send(Err(axum::Error::new(e))).await;
                    return;
                }
                None => {
                    // Stream ended — emit remaining buffer
                    if !buffer.is_empty() {
                        let transformed = transform_sse_chunk(&buffer, &original_model, &actual_model);
                        let _ = tx.send(Ok(Bytes::from(transformed))).await;
                    }
                    return;
                }
            }
        }
    });

    axum::body::Body::from_stream(ReceiverStream::new(rx))
}

fn find_sse_event_end(buffer: &[u8]) -> Option<usize> {
    for index in 0..buffer.len() {
        if buffer[index..].starts_with(b"\r\n\r\n") {
            return Some(index + 4);
        }
        if buffer[index..].starts_with(b"\n\n") {
            return Some(index + 2);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::{find_sse_event_end, transform_request_body, transform_sse_chunk};

    #[test]
    fn maps_all_claude_model_classes() {
        let cases = [
            ("claude-opus-4", "upstream-opus"),
            ("claude-sonnet-4", "upstream-sonnet"),
            ("claude-haiku-4", "upstream-haiku"),
            ("ccswitch-subagent", "upstream-subagent"),
        ];
        for (requested, expected) in cases {
            let body = format!(r#"{{"model":"{}"}}"#, requested);
            let (transformed, original, actual) = transform_request_body(body.as_bytes(), "upstream-opus", "upstream-sonnet", "upstream-haiku", "upstream-subagent").unwrap();
            let parsed: serde_json::Value = serde_json::from_slice(&transformed).unwrap();
            assert_eq!(original, requested);
            assert_eq!(actual, expected);
            assert_eq!(parsed["model"], expected);
        }
    }

    #[test]
    fn transforms_crlf_and_unterminated_sse_events() {
        assert_eq!(find_sse_event_end(b"data: {}\r\n\r\nrest"), Some(12));
        let input = br#"data: {"message":{"model":"upstream"}}"#;
        let output = transform_sse_chunk(input, "claude-sonnet", "actual-model");
        let text = String::from_utf8(output).unwrap();
        assert!(text.contains("claude-sonnet"));
        assert!(text.contains("actual-model"));
    }
}
