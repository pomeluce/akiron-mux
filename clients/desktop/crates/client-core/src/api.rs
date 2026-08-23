use std::time::Duration;

use reqwest::{Method, StatusCode};
use serde::{Serialize, de::DeserializeOwned};
use serde_json::{Value, json};
use url::Url;
use zeroize::Zeroizing;

use crate::{
    BackendHealth, BackendKind, BackendProfile, ClientError, CreateSessionRequest, DirectoryEntry, DirectoryListing, Project, Result, SessionDetails, SessionInfo,
    SettingsResponse, WorkspaceResponse,
};

const PROTOCOL_MAJOR: &str = "1";
const REQUIRED_REMOTE_CAPABILITIES: [&str; 3] = ["device-auth", "ws-ticket", "control-lease"];

#[derive(Debug, Clone)]
pub struct BackendApi {
    client: reqwest::Client,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct WebSocketTicket {
    pub ticket: String,
    pub expires_in_seconds: u8,
}

#[derive(Debug, serde::Deserialize)]
struct PairingResponse {
    token: String,
}

impl BackendApi {
    pub fn new() -> Result<Self> {
        let client = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(4))
            .timeout(Duration::from_secs(15))
            .build()
            .map_err(ClientError::Network)?;
        Ok(Self { client })
    }

    pub async fn health(&self, profile: &BackendProfile, credential: Option<&str>) -> Result<BackendHealth> {
        let value: Value = self.get(profile, credential, "/api/health").await?;
        let instance_id = value
            .get("instance_id")
            .and_then(Value::as_str)
            .ok_or_else(|| ClientError::Protocol("Backend identity is missing".into()))?;
        let api_protocol = value
            .get("api_protocol")
            .and_then(Value::as_str)
            .ok_or_else(|| ClientError::Protocol("Backend protocol is missing".into()))?;
        if api_protocol.split('.').next() != Some(PROTOCOL_MAJOR) {
            return Err(ClientError::Protocol(format!("Backend protocol {api_protocol} is incompatible")));
        }
        let capabilities = value
            .get("capabilities")
            .and_then(Value::as_array)
            .map(|items| items.iter().filter_map(Value::as_str).map(str::to_owned).collect())
            .unwrap_or_default();
        Ok(BackendHealth {
            instance_id: instance_id.into(),
            api_protocol: api_protocol.into(),
            capabilities,
        })
    }

    pub async fn pair(&self, profile: &BackendProfile, code: &str) -> Result<Zeroizing<String>> {
        let response: PairingResponse = self
            .send_json(
                profile,
                None,
                Method::POST,
                "/api/pair",
                Some(&json!({ "code": code, "device_name": "AkironMux Desktop" })),
                Duration::from_secs(70),
            )
            .await?;
        if response.token.is_empty() || response.token.len() > 16 * 1024 {
            return Err(ClientError::Protocol("Backend returned an invalid device credential".into()));
        }
        Ok(Zeroizing::new(response.token))
    }

    pub async fn revoke_current_device(&self, profile: &BackendProfile, credential: &str) -> Result<()> {
        self.send_empty(profile, Some(credential), Method::DELETE, "/api/device").await
    }

    pub async fn workspaces(&self, profile: &BackendProfile, credential: Option<&str>, query: Option<&str>) -> Result<WorkspaceResponse> {
        let path = query
            .filter(|query| !query.is_empty())
            .map(|query| format!("/api/workspaces?q={}", encode(query)))
            .unwrap_or_else(|| "/api/workspaces".into());
        self.get(profile, credential, &path).await
    }

    pub async fn refresh_history(&self, profile: &BackendProfile, credential: Option<&str>) -> Result<WorkspaceResponse> {
        self.post(profile, credential, "/api/history/refresh", &Value::Null).await
    }

    pub async fn settings(&self, profile: &BackendProfile, credential: Option<&str>) -> Result<SettingsResponse> {
        self.get(profile, credential, "/api/settings").await
    }

    pub async fn update_settings(&self, profile: &BackendProfile, credential: Option<&str>, patch: &Value) -> Result<SettingsResponse> {
        self.send_json(profile, credential, Method::PATCH, "/api/settings", Some(patch), Duration::from_secs(15))
            .await
    }

    pub async fn sessions(&self, profile: &BackendProfile, credential: Option<&str>) -> Result<Vec<SessionInfo>> {
        self.get(profile, credential, "/api/sessions").await
    }

    pub async fn session_details(&self, profile: &BackendProfile, credential: Option<&str>, id: &str) -> Result<SessionDetails> {
        self.get(profile, credential, &format!("/api/sessions/{}/details", encode(id))).await
    }

    pub async fn create_session(&self, profile: &BackendProfile, credential: Option<&str>, request: &CreateSessionRequest) -> Result<SessionInfo> {
        self.post(profile, credential, "/api/sessions", request).await
    }

    pub async fn restart_session(&self, profile: &BackendProfile, credential: Option<&str>, id: &str) -> Result<()> {
        self.send_empty(profile, credential, Method::POST, &format!("/api/sessions/{}/restart", encode(id))).await
    }

    pub async fn close_session(&self, profile: &BackendProfile, credential: Option<&str>, id: &str) -> Result<()> {
        self.send_empty(profile, credential, Method::DELETE, &format!("/api/sessions/{}", encode(id))).await
    }

    pub async fn websocket_ticket(&self, profile: &BackendProfile, credential: Option<&str>, session_id: &str) -> Result<WebSocketTicket> {
        self.post(profile, credential, "/api/auth/ws-ticket", &json!({ "session_id": session_id })).await
    }

    pub async fn directories(&self, profile: &BackendProfile, credential: Option<&str>, path: Option<&str>, show_hidden: bool) -> Result<DirectoryListing> {
        let path = match path.filter(|path| !path.is_empty()) {
            Some(path) => format!("/api/directories?path={}&show_hidden={show_hidden}", encode(path)),
            None => format!("/api/directories?show_hidden={show_hidden}"),
        };
        self.get(profile, credential, &path).await
    }

    pub async fn create_directory(&self, profile: &BackendProfile, credential: Option<&str>, parent: &str, name: &str) -> Result<DirectoryEntry> {
        self.post(profile, credential, "/api/directories", &json!({ "parent": parent, "name": name })).await
    }

    pub async fn create_project(&self, profile: &BackendProfile, credential: Option<&str>, path: &str, name: &str) -> Result<Project> {
        self.post(profile, credential, "/api/projects", &json!({ "path": path, "name": name })).await
    }

    pub async fn update_project(&self, profile: &BackendProfile, credential: Option<&str>, id: &str, patch: &Value) -> Result<Project> {
        self.send_json(
            profile,
            credential,
            Method::PATCH,
            &format!("/api/projects/{}", encode(id)),
            Some(patch),
            Duration::from_secs(15),
        )
        .await
    }

    pub async fn delete_project(&self, profile: &BackendProfile, credential: Option<&str>, id: &str) -> Result<()> {
        self.send_empty(profile, credential, Method::DELETE, &format!("/api/projects/{}", encode(id))).await
    }

    pub async fn reorder(&self, profile: &BackendProfile, credential: Option<&str>, kind: &str, scope: &str, ids: &[String]) -> Result<SettingsResponse> {
        self.post(profile, credential, "/api/reorder", &json!({ "kind": kind, "scope": scope, "ids": ids })).await
    }

    pub fn terminal_websocket_url(&self, profile: &BackendProfile, session_id: &str, ticket: Option<&str>) -> Result<Url> {
        let mut url = endpoint(profile, &format!("/api/sessions/{}/terminal", encode(session_id)))?;
        url.set_scheme(if url.scheme() == "https" { "wss" } else { "ws" })
            .map_err(|_| ClientError::Validation("Backend WebSocket URL is invalid".into()))?;
        if let Some(ticket) = ticket {
            url.query_pairs_mut().append_pair("ticket", ticket);
        }
        Ok(url)
    }

    async fn get<T: DeserializeOwned>(&self, profile: &BackendProfile, credential: Option<&str>, path: &str) -> Result<T> {
        self.send_json::<Value, T>(profile, credential, Method::GET, path, None, Duration::from_secs(15)).await
    }

    async fn post<B: Serialize + ?Sized, T: DeserializeOwned>(&self, profile: &BackendProfile, credential: Option<&str>, path: &str, body: &B) -> Result<T> {
        self.send_json(profile, credential, Method::POST, path, Some(body), Duration::from_secs(15)).await
    }

    async fn send_empty(&self, profile: &BackendProfile, credential: Option<&str>, method: Method, path: &str) -> Result<()> {
        let response = self.request::<Value>(profile, credential, method, path, None, Duration::from_secs(15)).await?;
        if response.status().is_success() {
            Ok(())
        } else {
            Err(status_error(response.status()).await)
        }
    }

    async fn send_json<B: Serialize + ?Sized, T: DeserializeOwned>(
        &self,
        profile: &BackendProfile,
        credential: Option<&str>,
        method: Method,
        path: &str,
        body: Option<&B>,
        timeout: Duration,
    ) -> Result<T> {
        let response = self.request(profile, credential, method, path, body, timeout).await?;
        if !response.status().is_success() {
            return Err(status_error(response.status()).await);
        }
        response.json().await.map_err(ClientError::Network)
    }

    async fn request<B: Serialize + ?Sized>(
        &self,
        profile: &BackendProfile,
        credential: Option<&str>,
        method: Method,
        path: &str,
        body: Option<&B>,
        timeout: Duration,
    ) -> Result<reqwest::Response> {
        let mut request = self
            .client
            .request(method, endpoint(profile, path)?)
            .header("X-Akmux-Protocol", PROTOCOL_MAJOR)
            .timeout(timeout);
        if profile.kind == BackendKind::Remote {
            let credential = credential.ok_or_else(|| ClientError::Authentication("Remote profile requires a device credential".into()))?;
            request = request.bearer_auth(credential);
        }
        if let Some(body) = body {
            request = request.json(body);
        }
        request.send().await.map_err(ClientError::Network)
    }
}

pub fn validate_profile(profile: &BackendProfile) -> Result<()> {
    if profile.id.is_empty()
        || profile.id.len() > 80
        || !profile
            .id
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || character == '-' || character == '_')
        || profile.name.trim().is_empty()
        || profile.name.chars().count() > 80
    {
        return Err(ClientError::Validation("Backend profile name and ID are invalid".into()));
    }
    let url = Url::parse(&profile.address).map_err(|_| ClientError::Validation("Backend address is invalid".into()))?;
    if url.path() != "/" || url.query().is_some() || url.fragment().is_some() || !url.username().is_empty() || url.password().is_some() {
        return Err(ClientError::Validation("Backend address cannot contain credentials, path, query, or fragment".into()));
    }
    match profile.kind {
        BackendKind::Local => {
            let loopback = match url.host() {
                Some(url::Host::Domain(host)) => host.eq_ignore_ascii_case("localhost"),
                Some(url::Host::Ipv4(address)) => address.is_loopback(),
                Some(url::Host::Ipv6(address)) => address.is_loopback(),
                None => false,
            };
            if url.scheme() != "http" || !loopback || profile.id != crate::LOCAL_PROFILE_ID || profile.name != "Local" {
                return Err(ClientError::Validation(
                    "The built-in Local profile requires its fixed name, ID, and an HTTP loopback address".into(),
                ));
            }
        }
        BackendKind::Remote if profile.id == crate::LOCAL_PROFILE_ID => {
            return Err(ClientError::Validation("The built-in Local profile cannot become Remote".into()));
        }
        BackendKind::Remote if url.scheme() == "https" => {}
        BackendKind::Remote => return Err(ClientError::Validation("Remote profiles require HTTPS".into())),
    }
    Ok(())
}

pub fn validate_remote_capabilities(capabilities: &[String]) -> Result<()> {
    let missing = REQUIRED_REMOTE_CAPABILITIES
        .iter()
        .filter(|required| !capabilities.iter().any(|capability| capability == **required))
        .copied()
        .collect::<Vec<_>>();
    if missing.is_empty() {
        Ok(())
    } else {
        Err(ClientError::Protocol(format!("Backend is missing required security capabilities: {}", missing.join(", "))))
    }
}

fn endpoint(profile: &BackendProfile, path: &str) -> Result<Url> {
    let base = Url::parse(&profile.address).map_err(|_| ClientError::Validation("Backend address is invalid".into()))?;
    base.join(path.trim_start_matches('/'))
        .map_err(|_| ClientError::Validation("Backend API URL is invalid".into()))
}

async fn status_error(status: StatusCode) -> ClientError {
    if status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN {
        ClientError::Authentication(format!("Backend rejected authentication (HTTP {})", status.as_u16()))
    } else {
        ClientError::Protocol(format!("Backend request failed (HTTP {})", status.as_u16()))
    }
}

fn encode(value: &str) -> String {
    url::form_urlencoded::byte_serialize(value.as_bytes()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_local_and_remote_profile_security_boundaries() {
        validate_profile(&BackendProfile::local()).unwrap();
        let mut remote = BackendProfile::local();
        remote.id = "remote".into();
        remote.name = "Remote".into();
        remote.kind = BackendKind::Remote;
        remote.address = "https://example.com".into();
        validate_profile(&remote).unwrap();
        remote.address = "http://example.com".into();
        assert!(validate_profile(&remote).is_err());
        remote.address = "https://user@example.com".into();
        assert!(validate_profile(&remote).is_err());
    }

    #[test]
    fn terminal_urls_use_the_matching_websocket_scheme_and_escape_ids() {
        let api = BackendApi::new().unwrap();
        let local = BackendProfile::local();
        assert_eq!(
            api.terminal_websocket_url(&local, "session/one", None).unwrap().as_str(),
            "ws://127.0.0.1:17321/api/sessions/session%2Fone/terminal"
        );
    }
}
