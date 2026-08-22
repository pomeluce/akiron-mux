use std::{
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};

use axum::{
    extract::ws::{Message, WebSocket, WebSocketUpgrade},
    response::Response,
};
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};

use crate::{
    db::Db,
    session_runtime::{SessionHandle, SessionInfo, SessionManager, SessionStreamEvent},
};

use super::remote::{self, ControlLeases, DeviceIdentity, LeaseState, RemoteSecurity};

const REMOTE_WEBSOCKET_LIMIT: usize = 32;
const IDLE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10 * 60);

#[derive(Clone)]
pub(super) struct TerminalHub {
    manager: SessionManager,
    db: Arc<Mutex<Db>>,
    security: RemoteSecurity,
    leases: ControlLeases,
    remote_websockets: Arc<tokio::sync::Semaphore>,
}

pub(super) enum TerminalAccess {
    Local(Option<DeviceIdentity>),
    Remote { ticket: Option<String> },
}

#[derive(Debug, thiserror::Error)]
pub(super) enum TerminalError {
    #[error("Managed session does not exist")]
    SessionNotFound,
    #[error("{0}")]
    Unauthorized(&'static str),
    #[error("Remote WebSocket limit reached")]
    Capacity,
}

impl TerminalHub {
    pub(super) fn new(manager: SessionManager, db: Arc<Mutex<Db>>, security: RemoteSecurity) -> Self {
        Self {
            manager,
            db,
            security,
            leases: ControlLeases::default(),
            remote_websockets: Arc::new(tokio::sync::Semaphore::new(REMOTE_WEBSOCKET_LIMIT)),
        }
    }
    pub(super) fn issue_ticket(&self, device: DeviceIdentity, session_id: String) -> Result<String, TerminalError> {
        if self.manager.get(&session_id).is_none() {
            return Err(TerminalError::SessionNotFound);
        }
        Ok(self.security.issue_ticket(device, session_id))
    }

    pub(super) fn upgrade(&self, ws: WebSocketUpgrade, session_id: String, access: TerminalAccess) -> Result<Response, TerminalError> {
        let session = self.manager.get(&session_id).ok_or(TerminalError::SessionNotFound)?;
        let (device, permit) = match access {
            TerminalAccess::Local(device) => (
                device.unwrap_or(DeviceIdentity {
                    token_id: "local".into(),
                    name: "Local client".into(),
                }),
                None,
            ),
            TerminalAccess::Remote { ticket } => {
                let ticket = ticket.as_deref().ok_or(TerminalError::Unauthorized("A WebSocket ticket is required"))?;
                let device = self
                    .security
                    .consume_ticket(ticket, &session_id)
                    .ok_or(TerminalError::Unauthorized("WebSocket ticket is invalid or expired"))?;
                if !self.device_is_active(&device) {
                    return Err(TerminalError::Unauthorized("Device credential has been revoked"));
                }
                let permit = self.remote_websockets.clone().try_acquire_owned().map_err(|_| TerminalError::Capacity)?;
                (device, Some(permit))
            }
        };
        let hub = self.clone();
        Ok(ws.on_upgrade(move |socket| hub.run(socket, session, session_id, device, permit)))
    }

    async fn run(self, socket: WebSocket, session: SessionHandle, session_id: String, device: DeviceIdentity, _permit: Option<tokio::sync::OwnedSemaphorePermit>) {
        let connection_id = remote::new_connection_id();
        let initial_lease = self.leases.connect(&session_id, &connection_id, &device.name);
        let _lease_guard = LeaseGuard::new(self.leases.clone(), session_id.clone(), connection_id.clone());
        let mut lease_events = self.leases.subscribe(&session_id);
        let (mut sender, mut receiver) = socket.split();

        if send_lease(&mut sender, &initial_lease.state, self.leases.can_write(&session_id, &connection_id))
            .await
            .is_err()
        {
            return;
        }
        if let Some(credential) = initial_lease.recovery_credential.as_deref() {
            if send_event(&mut sender, &ServerEvent::LeaseRecovery { credential }).await.is_err() {
                return;
            }
        }
        if send_replay(&mut sender, false, session.scrollback()).await.is_err() || send_status(&mut sender, &session.info()).await.is_err() {
            return;
        }

        let mut events = session.subscribe();
        let mut credential_check = tokio::time::interval(std::time::Duration::from_secs(1));
        let mut revocations = self.security.subscribe_revocations();
        let mut idle_check = tokio::time::interval(std::time::Duration::from_secs(30));
        let mut last_client_activity = std::time::Instant::now();

        loop {
            tokio::select! {
                incoming = receiver.next() => {
                    last_client_activity = std::time::Instant::now();
                    match incoming {
                        Some(Ok(Message::Binary(bytes))) => {
                            if self.leases.can_write(&session_id, &connection_id) && session.write(bytes.to_vec()).is_err() {
                                break;
                            }
                        }
                        Some(Ok(Message::Text(text))) => match parse_client_control(&text) {
                            ControlParse::Known(ClientControl::Resize { rows, cols }) => {
                                if self.leases.can_write(&session_id, &connection_id) && session.resize(rows, cols).is_err() {
                                    break;
                                }
                            }
                            ControlParse::Known(ClientControl::TakeControl { expected_version }) => {
                                let connection = self.leases.take_control(&session_id, &connection_id, &device.name, expected_version);
                                if let Some(credential) = connection.recovery_credential.as_deref() {
                                    if send_event(&mut sender, &ServerEvent::LeaseRecovery { credential }).await.is_err() {
                                        break;
                                    }
                                }
                            }
                            ControlParse::Known(ClientControl::RecoverControl { credential }) => {
                                let connection = self.leases.recover_control(&session_id, &connection_id, &device.name, &credential);
                                if let Some(credential) = connection.recovery_credential.as_deref() {
                                    if send_event(&mut sender, &ServerEvent::LeaseRecovery { credential }).await.is_err() {
                                        break;
                                    }
                                }
                            }
                            ControlParse::Unknown(kind) => tracing::debug!("Ignoring unknown terminal control message type '{kind}'"),
                            ControlParse::Invalid(message) => {
                                if send_event(
                                    &mut sender,
                                    &ServerEvent::ProtocolError {
                                        code: "invalid-control",
                                        message: &message,
                                    },
                                )
                                .await
                                .is_err()
                                {
                                    break;
                                }
                            }
                        },
                        Some(Ok(Message::Close(_))) | None | Some(Err(_)) => break,
                        Some(Ok(Message::Ping(bytes))) => {
                            if sender.send(Message::Pong(bytes)).await.is_err() {
                                break;
                            }
                        }
                        Some(Ok(Message::Pong(_))) => {}
                    }
                }
                event = events.recv() => {
                    match event {
                        Ok(SessionStreamEvent::Output(bytes)) => {
                            if sender.send(Message::Binary(bytes)).await.is_err() {
                                break;
                            }
                        }
                        Ok(SessionStreamEvent::Status(info)) => {
                            if send_status(&mut sender, &info).await.is_err() {
                                break;
                            }
                        }
                        Ok(SessionStreamEvent::Attention { kind, occurred_at_ms }) => {
                            if send_event(&mut sender, &ServerEvent::Attention { kind, occurred_at_ms }).await.is_err() {
                                break;
                            }
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                            if send_replay(&mut sender, true, session.scrollback()).await.is_err() {
                                break;
                            }
                        }
                    }
                }
                lease = lease_events.recv() => {
                    if let Ok(lease) = lease {
                        if send_lease(&mut sender, &lease, self.leases.can_write(&session_id, &connection_id)).await.is_err() {
                            break;
                        }
                    }
                }
                _ = credential_check.tick(), if device.token_id != "local" => {
                    if !self.device_is_active(&device) {
                        let _ = send_event(&mut sender, &ServerEvent::AuthorizationRevoked).await;
                        break;
                    }
                }
                revoked = revocations.recv(), if device.token_id != "local" => {
                    if revoked.ok().as_deref() == Some(device.token_id.as_str()) {
                        let _ = send_event(&mut sender, &ServerEvent::AuthorizationRevoked).await;
                        break;
                    }
                }
                _ = idle_check.tick() => {
                    if last_client_activity.elapsed() > IDLE_TIMEOUT {
                        let _ = sender.send(Message::Close(None)).await;
                        break;
                    }
                }
            }
        }
    }

    fn device_is_active(&self, device: &DeviceIdentity) -> bool {
        self.db
            .lock()
            .ok()
            .and_then(|db| db.backend_device_digest(&device.token_id).ok().flatten())
            .is_some_and(|(record, _)| record.revoked_at_ms.is_none())
    }
}

struct LeaseGuard {
    leases: ControlLeases,
    session_id: String,
    connection_id: String,
}

impl LeaseGuard {
    fn new(leases: ControlLeases, session_id: String, connection_id: String) -> Self {
        Self {
            leases,
            session_id,
            connection_id,
        }
    }
}

impl Drop for LeaseGuard {
    fn drop(&mut self) {
        self.leases.disconnect(&self.session_id, &self.connection_id);
    }
}

#[derive(Debug, Deserialize, PartialEq)]
#[serde(tag = "type")]
enum ClientControl {
    #[serde(rename = "resize")]
    Resize { rows: u16, cols: u16 },
    #[serde(rename = "take-control", alias = "takecontrol")]
    TakeControl { expected_version: u64 },
    #[serde(rename = "recover-control", alias = "recovercontrol")]
    RecoverControl { credential: String },
}

#[derive(Debug, PartialEq)]
enum ControlParse {
    Known(ClientControl),
    Unknown(String),
    Invalid(String),
}

fn parse_client_control(text: &str) -> ControlParse {
    let value: serde_json::Value = match serde_json::from_str(text) {
        Ok(value) => value,
        Err(error) => return ControlParse::Invalid(error.to_string()),
    };
    let Some(kind) = value.get("type").and_then(serde_json::Value::as_str) else {
        return ControlParse::Invalid("Terminal control message requires a string 'type'".into());
    };
    if !matches!(kind, "resize" | "take-control" | "takecontrol" | "recover-control" | "recovercontrol") {
        return ControlParse::Unknown(kind.to_string());
    }
    match serde_json::from_value(value) {
        Ok(control) => ControlParse::Known(control),
        Err(error) => ControlParse::Invalid(error.to_string()),
    }
}

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
enum ServerEvent<'a> {
    Replay {
        replace: bool,
    },
    Status {
        session: &'a SessionInfo,
        server_time_ms: u64,
    },
    Lease {
        lease: &'a LeaseState,
        can_write: bool,
    },
    LeaseRecovery {
        credential: &'a str,
    },
    Attention {
        kind: crate::session_runtime::AttentionKind,
        occurred_at_ms: u64,
    },
    AuthorizationRevoked,
    ProtocolError {
        code: &'a str,
        message: &'a str,
    },
}

async fn send_replay(sender: &mut futures_util::stream::SplitSink<WebSocket, Message>, replace: bool, scrollback: Vec<u8>) -> Result<(), axum::Error> {
    for frame in replay_frames(replace, scrollback) {
        sender.send(frame).await?;
    }
    Ok(())
}

fn replay_frames(replace: bool, scrollback: Vec<u8>) -> [Message; 2] {
    [
        Message::Text(serde_json::to_string(&ServerEvent::Replay { replace }).expect("terminal replay event serializes")),
        Message::Binary(scrollback),
    ]
}

async fn send_status(sender: &mut futures_util::stream::SplitSink<WebSocket, Message>, session: &SessionInfo) -> Result<(), axum::Error> {
    let server_time_ms = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis() as u64;
    send_event(sender, &ServerEvent::Status { session, server_time_ms }).await
}

async fn send_lease(sender: &mut futures_util::stream::SplitSink<WebSocket, Message>, lease: &LeaseState, can_write: bool) -> Result<(), axum::Error> {
    send_event(sender, &ServerEvent::Lease { lease, can_write }).await
}

async fn send_event(sender: &mut futures_util::stream::SplitSink<WebSocket, Message>, event: &ServerEvent<'_>) -> Result<(), axum::Error> {
    sender.send(Message::Text(serde_json::to_string(event).expect("terminal protocol events serialize"))).await
}

#[cfg(test)]
mod tests {
    use axum::extract::ws::Message;

    use super::{parse_client_control, replay_frames, ClientControl, ControlLeases, ControlParse, LeaseGuard, ServerEvent};

    #[test]
    fn parses_known_controls_and_distinguishes_unknown_from_invalid() {
        assert_eq!(
            parse_client_control(r#"{"type":"resize","rows":30,"cols":100}"#),
            ControlParse::Known(ClientControl::Resize { rows: 30, cols: 100 })
        );
        assert_eq!(
            parse_client_control(r#"{"type":"future-control","value":1}"#),
            ControlParse::Unknown("future-control".into())
        );
        assert!(matches!(parse_client_control(r#"{"type":"resize","rows":"large","cols":100}"#), ControlParse::Invalid(_)));
        assert!(matches!(parse_client_control("not json"), ControlParse::Invalid(_)));
    }

    #[test]
    fn replay_and_protocol_errors_have_stable_wire_names() {
        assert_eq!(
            serde_json::to_value(ServerEvent::Replay { replace: true }).unwrap(),
            serde_json::json!({ "type": "replay", "replace": true })
        );
        assert_eq!(
            serde_json::to_value(ServerEvent::ProtocolError {
                code: "invalid-control",
                message: "bad resize"
            })
            .unwrap(),
            serde_json::json!({ "type": "protocol-error", "code": "invalid-control", "message": "bad resize" })
        );
    }

    #[test]
    fn lease_guard_releases_the_connection_on_every_exit_path() {
        let leases = ControlLeases::default();
        let session_id = "session-1".to_string();
        let connection_id = "connection-1".to_string();
        leases.connect(&session_id, &connection_id, "Desktop");
        assert!(leases.can_write(&session_id, &connection_id));

        {
            let _guard = LeaseGuard::new(leases.clone(), session_id.clone(), connection_id.clone());
        }

        assert!(!leases.can_write(&session_id, &connection_id));
    }

    #[test]
    fn replay_marker_always_frames_exactly_one_binary_snapshot() {
        let [marker, snapshot] = replay_frames(true, Vec::new());
        assert_eq!(marker, Message::Text(r#"{"type":"replay","replace":true}"#.into()));
        assert_eq!(snapshot, Message::Binary(Vec::new()));
    }
}
