#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    #[error("{0}")]
    Validation(String),
    #[error("{0}")]
    Authentication(String),
    #[error("{0}")]
    Protocol(String),
    #[error("Unable to connect to the backend")]
    Network(#[source] reqwest::Error),
    #[error("Terminal connection failed")]
    WebSocket(#[source] Box<tokio_tungstenite::tungstenite::Error>),
    #[error("Unable to access client state: {0}")]
    Storage(#[source] std::io::Error),
    #[error("Client state is invalid: {0}")]
    InvalidState(#[source] serde_json::Error),
    #[error("{0}")]
    Platform(String),
}

pub type Result<T> = std::result::Result<T, ClientError>;
