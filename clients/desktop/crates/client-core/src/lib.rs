pub mod api;
pub mod credentials;
pub mod error;
pub mod lifecycle;
pub mod models;
pub mod storage;
pub mod terminal_protocol;
pub mod terminal_transport;

pub use api::{BackendApi, WebSocketTicket, validate_profile, validate_remote_capabilities};
pub use credentials::{CredentialStore, MemoryCredentialStore};
pub use error::{ClientError, Result};
pub use lifecycle::{BackendLifecycleOutcome, BackendProfileIntent, BackendProfileLifecycle};
pub use models::*;
pub use storage::{ClientPaths, ClientStateStore, operation_id, save_json_atomically};
pub use terminal_protocol::{
    ClientControl, LeaseState, MemoryRecoveryCredentialStore, RecoveryCredentialStore, TerminalCommand, TerminalConnectionPhase, TerminalEvent, TerminalProtocolState,
    reconnect_delay_ms,
};
pub use terminal_transport::{TerminalTransport, TerminalTransportConfig};
