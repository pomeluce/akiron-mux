use std::{collections::HashMap, sync::Mutex};

use serde::{Deserialize, Serialize};

use crate::{AttentionKind, ClientError, Result, SessionInfo};

pub const ATTENTION_MAX_DELIVERY_AGE_MS: i64 = 30_000;
pub const RECONNECT_BASE_DELAY_MS: u64 = 1_200;
pub const RECONNECT_MAX_DELAY_MS: u64 = 30_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TerminalConnectionPhase {
    RequestingTicket,
    Connecting,
    Open,
    Reconnecting,
    Revoked,
    Disposed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LeaseState {
    pub version: u64,
    pub controller_device_name: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TerminalEvent {
    Output { bytes: Vec<u8>, replace: bool },
    Status(SessionInfo),
    Attention(AttentionKind),
    Lease { lease: LeaseState, can_write: bool },
    RecoveryCredential(String),
    AuthorizationRevoked,
    ProtocolError { code: Option<String>, message: String },
    Phase(TerminalConnectionPhase),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TerminalCommand {
    Input(Vec<u8>),
    Resize { rows: u16, cols: u16 },
    TakeControl { expected_version: u64 },
    Disconnect,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum ClientControl<'a> {
    Resize { rows: u16, cols: u16 },
    TakeControl { expected_version: u64 },
    RecoverControl { credential: &'a str },
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
enum ServerControl {
    Replay { replace: bool },
    Reset,
    Status { session: SessionInfo, server_time_ms: Option<i64> },
    Attention { kind: AttentionKind, occurred_at_ms: Option<i64> },
    Lease { lease: LeaseState, can_write: bool },
    LeaseRecovery { credential: String },
    AuthorizationRevoked,
    ProtocolError { code: Option<String>, message: String },
}

#[derive(Debug, Default)]
pub struct TerminalProtocolState {
    pending_replay: Option<bool>,
    server_clock_offset_ms: Option<i64>,
    lease: Option<LeaseState>,
    can_write: bool,
    revoked: bool,
}

impl TerminalProtocolState {
    pub fn reset_for_connection(&mut self) {
        self.pending_replay = None;
        self.server_clock_offset_ms = None;
        self.can_write = false;
        self.revoked = false;
    }

    pub fn can_write(&self) -> bool {
        self.can_write && !self.revoked
    }

    pub fn lease(&self) -> Option<&LeaseState> {
        self.lease.as_ref()
    }

    pub fn revoked(&self) -> bool {
        self.revoked
    }

    pub fn handle_text(&mut self, text: &str, client_now_ms: i64) -> Result<Vec<TerminalEvent>> {
        let control: ServerControl = serde_json::from_str(text).map_err(|error| ClientError::Protocol(format!("Server sent invalid terminal protocol JSON: {error}")))?;
        let events = match control {
            ServerControl::Replay { replace } => {
                self.pending_replay = Some(replace);
                Vec::new()
            }
            ServerControl::Reset => {
                self.pending_replay = Some(true);
                Vec::new()
            }
            ServerControl::Status { session, server_time_ms } => {
                if let Some(server_time_ms) = server_time_ms {
                    self.server_clock_offset_ms = Some(client_now_ms.saturating_sub(server_time_ms));
                }
                vec![TerminalEvent::Status(session)]
            }
            ServerControl::Attention { kind, occurred_at_ms } => {
                if self.attention_is_fresh(occurred_at_ms, client_now_ms) {
                    vec![TerminalEvent::Attention(kind)]
                } else {
                    Vec::new()
                }
            }
            ServerControl::Lease { lease, can_write } => {
                self.lease = Some(lease.clone());
                self.can_write = can_write;
                vec![TerminalEvent::Lease { lease, can_write }]
            }
            ServerControl::LeaseRecovery { credential } => {
                if credential.is_empty() {
                    return Err(ClientError::Protocol("Lease recovery message requires a credential".into()));
                }
                vec![TerminalEvent::RecoveryCredential(credential)]
            }
            ServerControl::AuthorizationRevoked => {
                self.revoked = true;
                self.can_write = false;
                vec![TerminalEvent::AuthorizationRevoked, TerminalEvent::Phase(TerminalConnectionPhase::Revoked)]
            }
            ServerControl::ProtocolError { code, message } => vec![TerminalEvent::ProtocolError { code, message }],
        };
        Ok(events)
    }

    pub fn handle_binary(&mut self, bytes: Vec<u8>) -> TerminalEvent {
        TerminalEvent::Output {
            bytes,
            replace: self.pending_replay.take().unwrap_or(false),
        }
    }

    fn attention_is_fresh(&self, occurred_at_ms: Option<i64>, client_now_ms: i64) -> bool {
        match (occurred_at_ms, self.server_clock_offset_ms) {
            (Some(occurred_at_ms), Some(offset)) => client_now_ms.saturating_sub(occurred_at_ms.saturating_add(offset)) <= ATTENTION_MAX_DELIVERY_AGE_MS,
            _ => true,
        }
    }
}

pub trait RecoveryCredentialStore: Send + Sync {
    fn load(&self, backend_key: &str, session_id: &str) -> Option<String>;
    fn store(&self, backend_key: &str, session_id: &str, credential: String);
    fn remove(&self, backend_key: &str, session_id: &str);
}

#[derive(Default)]
pub struct MemoryRecoveryCredentialStore(Mutex<HashMap<(String, String), String>>);

impl RecoveryCredentialStore for MemoryRecoveryCredentialStore {
    fn load(&self, backend_key: &str, session_id: &str) -> Option<String> {
        self.0.lock().ok()?.get(&(backend_key.into(), session_id.into())).cloned()
    }

    fn store(&self, backend_key: &str, session_id: &str, credential: String) {
        if let Ok(mut values) = self.0.lock() {
            values.insert((backend_key.into(), session_id.into()), credential);
        }
    }

    fn remove(&self, backend_key: &str, session_id: &str) {
        if let Ok(mut values) = self.0.lock() {
            values.remove(&(backend_key.into(), session_id.into()));
        }
    }
}

pub fn reconnect_delay_ms(attempt: u32) -> u64 {
    RECONNECT_BASE_DELAY_MS.saturating_mul(2_u64.saturating_pow(attempt)).min(RECONNECT_MAX_DELAY_MS)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session() -> SessionInfo {
        SessionInfo {
            id: "session".into(),
            agent: crate::Agent::Codex,
            title: "Test".into(),
            cwd: "/tmp".into(),
            status: crate::SessionStatus::Running,
            created_at_ms: 1,
            exit_code: None,
            error: None,
            native_session_id: None,
        }
    }

    #[test]
    fn replacement_replay_marks_exactly_the_next_binary_frame() {
        let mut state = TerminalProtocolState::default();
        assert!(state.handle_text(r#"{"type":"replay","replace":true}"#, 0).unwrap().is_empty());
        assert_eq!(
            state.handle_binary(b"snapshot".to_vec()),
            TerminalEvent::Output {
                bytes: b"snapshot".to_vec(),
                replace: true
            }
        );
        assert_eq!(
            state.handle_binary(b"next".to_vec()),
            TerminalEvent::Output {
                bytes: b"next".to_vec(),
                replace: false
            }
        );
    }

    #[test]
    fn stale_attention_is_ignored_after_clock_offset_is_known() {
        let mut state = TerminalProtocolState::default();
        let status = serde_json::json!({ "type": "status", "session": session(), "server_time_ms": 10_000 });
        state.handle_text(&status.to_string(), 20_000).unwrap();
        let stale = serde_json::json!({ "type": "attention", "kind": "input", "occurred_at_ms": -30_001 });
        assert!(state.handle_text(&stale.to_string(), 20_000).unwrap().is_empty());
        let fresh = serde_json::json!({ "type": "attention", "kind": "completed", "occurred_at_ms": 9_999 });
        assert_eq!(
            state.handle_text(&fresh.to_string(), 20_000).unwrap(),
            vec![TerminalEvent::Attention(AttentionKind::Completed)]
        );
    }

    #[test]
    fn revocation_disables_input_and_exposes_a_terminal_phase() {
        let mut state = TerminalProtocolState::default();
        state
            .handle_text(r#"{"type":"lease","lease":{"version":2,"controller_device_name":"Desktop"},"can_write":true}"#, 0)
            .unwrap();
        assert!(state.can_write());
        let events = state.handle_text(r#"{"type":"authorization-revoked"}"#, 0).unwrap();
        assert!(!state.can_write());
        assert!(state.revoked());
        assert!(events.contains(&TerminalEvent::Phase(TerminalConnectionPhase::Revoked)));
    }

    #[test]
    fn reconnect_backoff_caps_at_thirty_seconds() {
        assert_eq!(reconnect_delay_ms(0), 1_200);
        assert_eq!(reconnect_delay_ms(1), 2_400);
        assert_eq!(reconnect_delay_ms(20), 30_000);
    }
}
