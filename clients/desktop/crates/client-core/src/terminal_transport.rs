use std::{
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use async_channel::{Receiver, Sender};
use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::{connect_async, tungstenite::Message};
use zeroize::Zeroizing;

use crate::{
    BackendApi, BackendKind, BackendProfile, ClientControl, ClientError, MemoryRecoveryCredentialStore, RecoveryCredentialStore, Result, TerminalCommand, TerminalConnectionPhase,
    TerminalEvent, TerminalProtocolState, reconnect_delay_ms,
};

const CHANNEL_CAPACITY: usize = 256;

#[derive(Clone)]
pub struct TerminalTransportConfig {
    pub api: BackendApi,
    pub profile: BackendProfile,
    pub credential: Option<Arc<Zeroizing<String>>>,
    pub session_id: String,
    pub recovery_store: Arc<dyn RecoveryCredentialStore>,
}

impl TerminalTransportConfig {
    pub fn local(session_id: impl Into<String>) -> Result<Self> {
        Ok(Self {
            api: BackendApi::new()?,
            profile: BackendProfile::local(),
            credential: None,
            session_id: session_id.into(),
            recovery_store: Arc::new(MemoryRecoveryCredentialStore::default()),
        })
    }
}

pub struct TerminalTransport {
    commands: Sender<TerminalCommand>,
    events: Receiver<TerminalEvent>,
}

impl TerminalTransport {
    pub fn spawn(config: TerminalTransportConfig, runtime: &tokio::runtime::Handle) -> Self {
        let (commands_tx, commands_rx) = async_channel::bounded(CHANNEL_CAPACITY);
        let (events_tx, events_rx) = async_channel::bounded(CHANNEL_CAPACITY);
        runtime.spawn(run(config, commands_rx, events_tx));
        Self {
            commands: commands_tx,
            events: events_rx,
        }
    }

    pub fn commands(&self) -> Sender<TerminalCommand> {
        self.commands.clone()
    }

    pub fn events(&self) -> Receiver<TerminalEvent> {
        self.events.clone()
    }

    pub fn disconnect(&self) {
        let _ = self.commands.try_send(TerminalCommand::Disconnect);
    }
}

impl Drop for TerminalTransport {
    fn drop(&mut self) {
        self.disconnect();
    }
}

async fn run(config: TerminalTransportConfig, commands: Receiver<TerminalCommand>, events: Sender<TerminalEvent>) {
    let mut attempts = 0;
    let mut pending_resize = None;

    loop {
        if commands.is_closed() {
            break;
        }

        let ticket = if config.profile.kind == BackendKind::Remote {
            if send_event(&events, TerminalEvent::Phase(TerminalConnectionPhase::RequestingTicket)).await.is_err() {
                break;
            }
            match config
                .api
                .websocket_ticket(&config.profile, config.credential.as_deref().map(|credential| credential.as_str()), &config.session_id)
                .await
            {
                Ok(ticket) => Some(ticket.ticket),
                Err(error) => {
                    if reconnect(&commands, &events, &mut attempts, error).await {
                        continue;
                    }
                    break;
                }
            }
        } else {
            None
        };

        let phase = if attempts == 0 {
            TerminalConnectionPhase::Connecting
        } else {
            TerminalConnectionPhase::Reconnecting
        };
        if send_event(&events, TerminalEvent::Phase(phase)).await.is_err() {
            break;
        }
        let url = match config.api.terminal_websocket_url(&config.profile, &config.session_id, ticket.as_deref()) {
            Ok(url) => url,
            Err(error) => {
                if reconnect(&commands, &events, &mut attempts, error).await {
                    continue;
                }
                break;
            }
        };
        let (mut socket, _) = match connect_async(url.as_str()).await {
            Ok(socket) => socket,
            Err(error) => {
                if reconnect(&commands, &events, &mut attempts, ClientError::WebSocket(Box::new(error))).await {
                    continue;
                }
                break;
            }
        };

        attempts = 0;
        let mut protocol = TerminalProtocolState::default();
        protocol.reset_for_connection();
        if send_event(&events, TerminalEvent::Phase(TerminalConnectionPhase::Open)).await.is_err() {
            break;
        }

        if let Some(credential) = config.recovery_store.load(&config.profile.id, &config.session_id)
            && send_control(&mut socket, &ClientControl::RecoverControl { credential: &credential }).await.is_err()
        {
            continue;
        }

        let mut reconnect_socket = false;
        loop {
            tokio::select! {
                command = commands.recv() => {
                    let Ok(command) = command else { break; };
                    match command {
                        TerminalCommand::Disconnect => {
                            let _ = socket.close(None).await;
                            let _ = send_event(&events, TerminalEvent::Phase(TerminalConnectionPhase::Disposed)).await;
                            return;
                        }
                        TerminalCommand::Input(bytes) if protocol.can_write() => {
                            if socket.send(Message::Binary(bytes.into())).await.is_err() {
                                reconnect_socket = true;
                                break;
                            }
                        }
                        TerminalCommand::Resize { rows, cols } => {
                            pending_resize = Some((rows, cols));
                            if protocol.can_write() && send_control(&mut socket, &ClientControl::Resize { rows, cols }).await.is_err() {
                                reconnect_socket = true;
                                break;
                            }
                        }
                        TerminalCommand::TakeControl { expected_version } => {
                            if send_control(&mut socket, &ClientControl::TakeControl { expected_version }).await.is_err() {
                                reconnect_socket = true;
                                break;
                            }
                        }
                        TerminalCommand::Input(_) => {}
                    }
                }
                message = socket.next() => {
                    match message {
                        Some(Ok(Message::Text(text))) => {
                            match protocol.handle_text(text.as_str(), now_ms()) {
                                Ok(parsed) => {
                                    for event in parsed {
                                        match &event {
                                            TerminalEvent::RecoveryCredential(credential) => config.recovery_store.store(&config.profile.id, &config.session_id, credential.clone()),
                                            TerminalEvent::Lease { can_write: false, .. } | TerminalEvent::AuthorizationRevoked => config.recovery_store.remove(&config.profile.id, &config.session_id),
                                            _ => {}
                                        }
                                        let revoked = matches!(event, TerminalEvent::AuthorizationRevoked);
                                        let writable = matches!(event, TerminalEvent::Lease { can_write: true, .. });
                                        if send_event(&events, event).await.is_err() { return; }
                                        if writable
                                            && let Some((rows, cols)) = pending_resize
                                            && send_control(&mut socket, &ClientControl::Resize { rows, cols }).await.is_err()
                                        {
                                            reconnect_socket = true;
                                            break;
                                        }
                                        if revoked {
                                            let _ = socket.close(None).await;
                                            return;
                                        }
                                    }
                                    if reconnect_socket { break; }
                                }
                                Err(error) => {
                                    let _ = send_event(&events, TerminalEvent::ProtocolError { code: None, message: error.to_string() }).await;
                                }
                            }
                        }
                        Some(Ok(Message::Binary(bytes))) => {
                            if send_event(&events, protocol.handle_binary(bytes.to_vec())).await.is_err() { return; }
                        }
                        Some(Ok(Message::Ping(bytes))) => {
                            if socket.send(Message::Pong(bytes)).await.is_err() { reconnect_socket = true; break; }
                        }
                        Some(Ok(Message::Close(_))) | None | Some(Err(_)) => { reconnect_socket = true; break; }
                        Some(Ok(_)) => {}
                    }
                }
            }
        }

        if !reconnect_socket {
            break;
        }
        if !wait_for_reconnect(&commands, &events, &mut attempts).await {
            break;
        }
    }
    let _ = send_event(&events, TerminalEvent::Phase(TerminalConnectionPhase::Disposed)).await;
}

async fn send_control<S>(socket: &mut S, control: &ClientControl<'_>) -> Result<()>
where
    S: futures_util::Sink<Message, Error = tokio_tungstenite::tungstenite::Error> + Unpin,
{
    let text = serde_json::to_string(control).map_err(ClientError::InvalidState)?;
    socket.send(Message::Text(text.into())).await.map_err(|error| ClientError::WebSocket(Box::new(error)))
}

async fn send_event(events: &Sender<TerminalEvent>, event: TerminalEvent) -> std::result::Result<(), async_channel::SendError<TerminalEvent>> {
    events.send(event).await
}

async fn reconnect(commands: &Receiver<TerminalCommand>, events: &Sender<TerminalEvent>, attempts: &mut u32, error: ClientError) -> bool {
    if send_event(
        events,
        TerminalEvent::ProtocolError {
            code: None,
            message: error.to_string(),
        },
    )
    .await
    .is_err()
    {
        return false;
    }
    wait_for_reconnect(commands, events, attempts).await
}

async fn wait_for_reconnect(commands: &Receiver<TerminalCommand>, events: &Sender<TerminalEvent>, attempts: &mut u32) -> bool {
    if send_event(events, TerminalEvent::Phase(TerminalConnectionPhase::Reconnecting)).await.is_err() {
        return false;
    }
    let delay = reconnect_delay_ms(*attempts);
    *attempts = attempts.saturating_add(1);
    tokio::select! {
        _ = tokio::time::sleep(Duration::from_millis(delay)) => true,
        command = commands.recv() => !matches!(command, Ok(TerminalCommand::Disconnect) | Err(_)),
    }
}

fn now_ms() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis().min(i64::MAX as u128) as i64
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::net::TcpListener;
    use tokio_tungstenite::accept_async;

    #[tokio::test]
    async fn local_transport_replays_output_and_gates_resize_on_the_lease() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = accept_async(stream).await.unwrap();
            socket
                .send(Message::Text(
                    serde_json::json!({
                        "type": "lease",
                        "lease": { "version": 7, "controller_device_name": "AkironMux Desktop" },
                        "can_write": true
                    })
                    .to_string()
                    .into(),
                ))
                .await
                .unwrap();
            socket.send(Message::Text(r#"{"type":"replay","replace":true}"#.into())).await.unwrap();
            socket.send(Message::Binary(b"snapshot".to_vec().into())).await.unwrap();
            let resize = socket.next().await.unwrap().unwrap();
            let input = socket.next().await.unwrap().unwrap();
            (resize, input)
        });

        let mut profile = BackendProfile::local();
        profile.address = format!("http://{address}");
        let config = TerminalTransportConfig {
            api: BackendApi::new().unwrap(),
            profile,
            credential: None,
            session_id: "session".into(),
            recovery_store: Arc::new(MemoryRecoveryCredentialStore::default()),
        };
        let transport = TerminalTransport::spawn(config, &tokio::runtime::Handle::current());
        let commands = transport.commands();
        commands.send(TerminalCommand::Resize { rows: 40, cols: 120 }).await.unwrap();

        let mut saw_lease = false;
        let mut saw_replay = false;
        for _ in 0..8 {
            let event = tokio::time::timeout(Duration::from_secs(2), transport.events.recv()).await.unwrap().unwrap();
            match event {
                TerminalEvent::Lease { lease, can_write } => {
                    assert_eq!(lease.version, 7);
                    assert!(can_write);
                    saw_lease = true;
                    commands.send(TerminalCommand::Input(b"input".to_vec())).await.unwrap();
                }
                TerminalEvent::Output { bytes, replace } => {
                    assert_eq!(bytes, b"snapshot");
                    assert!(replace);
                    saw_replay = true;
                }
                _ => {}
            }
            if saw_lease && saw_replay {
                break;
            }
        }
        assert!(saw_lease && saw_replay);
        let (resize, input) = tokio::time::timeout(Duration::from_secs(2), server).await.unwrap().unwrap();
        assert_eq!(resize.into_text().unwrap(), r#"{"type":"resize","rows":40,"cols":120}"#);
        assert_eq!(input.into_data().as_ref(), b"input");
    }

    #[tokio::test]
    async fn sustained_output_preserves_every_binary_frame_and_byte() {
        const FRAME_COUNT: usize = 512;
        const FRAME_SIZE: usize = 4 * 1024;

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let expected = (0..FRAME_COUNT)
            .flat_map(|frame| (0..FRAME_SIZE).map(move |offset| ((frame * 31 + offset) % 251) as u8))
            .collect::<Vec<_>>();
        let server_bytes = expected.clone();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = accept_async(stream).await.unwrap();
            for frame in server_bytes.chunks_exact(FRAME_SIZE) {
                socket.send(Message::Binary(frame.to_vec().into())).await.unwrap();
            }
            let _ = socket.next().await;
        });

        let mut profile = BackendProfile::local();
        profile.address = format!("http://{address}");
        let transport = TerminalTransport::spawn(
            TerminalTransportConfig {
                api: BackendApi::new().unwrap(),
                profile,
                credential: None,
                session_id: "throughput".into(),
                recovery_store: Arc::new(MemoryRecoveryCredentialStore::default()),
            },
            &tokio::runtime::Handle::current(),
        );

        let mut actual = Vec::with_capacity(expected.len());
        let receive = async {
            while actual.len() < expected.len() {
                if let TerminalEvent::Output { bytes, replace } = transport.events.recv().await.unwrap() {
                    assert!(!replace);
                    actual.extend_from_slice(&bytes);
                }
            }
        };
        tokio::time::timeout(Duration::from_secs(10), receive).await.unwrap();
        assert_eq!(actual, expected);
        transport.disconnect();
        tokio::time::timeout(Duration::from_secs(2), server).await.unwrap().unwrap();
    }
}
