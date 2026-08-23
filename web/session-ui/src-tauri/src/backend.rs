use std::{
    collections::HashMap,
    io::Write,
    path::PathBuf,
    sync::{
        atomic::{AtomicU64, Ordering},
        Mutex, OnceLock,
    },
};

use reqwest::Method;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};
use url::Url;
use zeroize::Zeroizing;

mod lifecycle;

pub use lifecycle::{BackendLifecycleOutcome, BackendProfileIntent};

const LOCAL_PROFILE_ID: &str = "local";
const PROTOCOL_MAJOR: &str = "1";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BackendProfile {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub address: String,
    #[serde(default)]
    pub instance_id: Option<String>,
    #[serde(default)]
    pub has_credential: bool,
    #[serde(default)]
    pub requires_auth: bool,
    #[serde(default)]
    pub capabilities: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BackendProfileState {
    pub profiles: Vec<BackendProfile>,
    pub active_profile_id: String,
}

impl Default for BackendProfileState {
    fn default() -> Self {
        Self {
            profiles: vec![BackendProfile {
                id: LOCAL_PROFILE_ID.into(),
                name: "Local".into(),
                kind: "local".into(),
                address: "http://127.0.0.1:17321".into(),
                instance_id: None,
                has_credential: false,
                requires_auth: false,
                capabilities: Vec::new(),
            }],
            active_profile_id: LOCAL_PROFILE_ID.into(),
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BackendHealth {
    pub instance_id: String,
    pub api_protocol: String,
    pub capabilities: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BackendRequest {
    pub profile_id: String,
    pub method: String,
    pub path: String,
    pub body: Option<serde_json::Value>,
}

#[derive(Debug, Serialize)]
pub struct BackendResponse {
    pub status: u16,
    pub body: serde_json::Value,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PersistedBackendProfile {
    id: String,
    name: String,
    kind: String,
    address: String,
    #[serde(default)]
    instance_id: Option<String>,
    #[serde(default)]
    capabilities: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PersistedBackendProfileState {
    profiles: Vec<PersistedBackendProfile>,
    active_profile_id: String,
}

impl From<&BackendProfile> for PersistedBackendProfile {
    fn from(profile: &BackendProfile) -> Self {
        Self {
            id: profile.id.clone(),
            name: profile.name.clone(),
            kind: profile.kind.clone(),
            address: profile.address.clone(),
            instance_id: profile.instance_id.clone(),
            capabilities: profile.capabilities.clone(),
        }
    }
}

const REQUIRED_REMOTE_CAPABILITIES: [&str; 3] = ["device-auth", "ws-ticket", "control-lease"];

#[derive(Debug, Deserialize)]
struct PairingResponse {
    token: String,
}

struct ParsedPairingLink {
    code: String,
}

trait CredentialStore {
    fn store(&self, profile_id: &str, token: &str) -> Result<(), String>;
    fn load(&self, profile_id: &str) -> Result<Zeroizing<String>, String>;
    fn delete(&self, profile_id: &str) -> Result<(), String>;
}

struct PlatformCredentialStore;

impl CredentialStore for PlatformCredentialStore {
    fn store(&self, profile_id: &str, token: &str) -> Result<(), String> {
        platform_store_credential(profile_id, token)
    }

    fn load(&self, profile_id: &str) -> Result<Zeroizing<String>, String> {
        platform_load_credential(profile_id)
    }

    fn delete(&self, profile_id: &str) -> Result<(), String> {
        platform_delete_credential(profile_id)
    }
}

fn store_credential(profile_id: &str, token: &str) -> Result<(), String> {
    PlatformCredentialStore.store(profile_id, token)
}

fn load_credential(profile_id: &str) -> Result<Zeroizing<String>, String> {
    PlatformCredentialStore.load(profile_id)
}

fn delete_credential(profile_id: &str) -> Result<(), String> {
    PlatformCredentialStore.delete(profile_id)
}

#[tauri::command]
pub fn list_backend_profiles(app: AppHandle) -> Result<BackendProfileState, String> {
    load_state(&app)
}

#[tauri::command]
pub async fn apply_backend_profile_intent(app: AppHandle, intent: BackendProfileIntent) -> Result<BackendLifecycleOutcome, String> {
    lifecycle::BackendProfileLifecycle::new(app).apply(intent).await
}

#[tauri::command]
pub async fn test_backend_profile(app: AppHandle, profile: BackendProfile) -> Result<BackendHealth, String> {
    validate_profile(&profile)?;
    let state = load_state(&app)?;
    let credential = credential_for_unchanged_profile(&state, &profile);
    let health = request_health(&profile, credential.as_deref().map(String::as_str)).await?;
    if profile.kind == "remote" {
        validate_remote_capabilities(&health.capabilities)?;
    }
    Ok(health)
}

fn parse_pairing_link(value: &str, expected_address: &str) -> Result<ParsedPairingLink, String> {
    let link = Url::parse(value.trim()).map_err(|_| "Pairing link is invalid".to_string())?;
    if link.scheme() != "akmux" || link.host_str() != Some("pair") || link.path() != "" {
        return Err("Pairing link is invalid".into());
    }
    let mut address = None;
    let mut code = None;
    for (key, value) in link.query_pairs() {
        match key.as_ref() {
            "url" if address.is_none() => address = Some(value.into_owned()),
            "code" if code.is_none() => code = Some(value.into_owned()),
            _ => {}
        }
    }
    let address = Url::parse(address.as_deref().ok_or_else(|| "Pairing link is missing its backend address".to_string())?)
        .map_err(|_| "Pairing link contains an invalid backend address".to_string())?;
    let expected = Url::parse(expected_address).map_err(|_| "Backend address is invalid".to_string())?;
    if address != expected {
        return Err("Pairing link belongs to a different backend".into());
    }
    let code = code
        .filter(|value| !value.is_empty() && value.len() <= 256)
        .ok_or_else(|| "Pairing link is missing its code".to_string())?;
    Ok(ParsedPairingLink { code })
}

async fn request_pairing_token(profile: &BackendProfile, code: &str) -> Result<Zeroizing<String>, String> {
    let client = reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(4))
        .timeout(std::time::Duration::from_secs(70))
        .build()
        .map_err(|_| "Unable to initialize backend networking".to_string())?;
    let response = client
        .post(api_url(profile, "/api/pair")?)
        .json(&serde_json::json!({ "code": code, "device_name": "AkironMux Desktop" }))
        .send()
        .await
        .map_err(safe_network_error)?;
    if !response.status().is_success() {
        return Err(format!("Backend rejected pairing (HTTP {})", response.status().as_u16()));
    }
    let response = response.json::<PairingResponse>().await.map_err(safe_network_error)?;
    if response.token.is_empty() || response.token.len() > 16 * 1024 {
        return Err("Backend returned an invalid device credential".into());
    }
    Ok(Zeroizing::new(response.token))
}

#[tauri::command]
pub async fn backend_request(app: AppHandle, request: BackendRequest) -> Result<BackendResponse, String> {
    let method = Method::from_bytes(request.method.as_bytes()).map_err(|_| "Backend HTTP method is invalid".to_string())?;
    if !is_allowed_api_request(&method, &request.path) {
        return Err("Backend API path is not allowed".into());
    }
    let state = load_state(&app)?;
    if state.active_profile_id != request.profile_id {
        return Err("Backend profile is not active".into());
    }
    let profile = state
        .profiles
        .into_iter()
        .find(|profile| profile.id == request.profile_id)
        .ok_or_else(|| "Backend profile does not exist".to_string())?;
    if profile.kind == "remote" {
        let token = load_credential(&profile.id)?;
        let health = request_health(&profile, Some(token.as_str())).await?;
        if profile.instance_id.as_deref() != Some(health.instance_id.as_str()) {
            return Err("BACKEND_IDENTITY_CONFIRMATION_REQUIRED".into());
        }
        validate_remote_capabilities(&health.capabilities)?;
    }
    let url = api_url(&profile, &request.path)?;
    let client = http_client()?;
    let mut builder = client.request(method, url).header("X-Akmux-Protocol", PROTOCOL_MAJOR);
    if profile.kind == "remote" {
        let token = load_credential(&profile.id)?;
        builder = builder.bearer_auth(token.as_str());
    }
    if let Some(body) = request.body {
        builder = builder.json(&body);
    }
    let response = builder.send().await.map_err(safe_network_error)?;
    let status = response.status();
    let bytes = response.bytes().await.map_err(safe_network_error)?;
    let body = if bytes.is_empty() {
        serde_json::Value::Null
    } else {
        serde_json::from_slice(&bytes).map_err(|_| "Backend returned an invalid response".to_string())?
    };
    Ok(BackendResponse { status: status.as_u16(), body })
}

fn is_allowed_api_request(method: &Method, path: &str) -> bool {
    if !path.starts_with("/api/") || path.contains("..") {
        return false;
    }
    let route = path.split('?').next().unwrap_or(path);
    (method == Method::GET && matches!(route, "/api/workspaces" | "/api/settings" | "/api/sessions" | "/api/directories"))
        || (method == Method::POST
            && matches!(
                route,
                "/api/history/refresh" | "/api/sessions" | "/api/directories" | "/api/reorder" | "/api/auth/ws-ticket" | "/api/projects"
            ))
        || (method == Method::PATCH && route == "/api/settings")
        || (method == Method::GET && route.starts_with("/api/sessions/") && route.ends_with("/details"))
        || (method == Method::POST && route.starts_with("/api/sessions/") && route.ends_with("/restart"))
        || (method == Method::DELETE && route.starts_with("/api/sessions/"))
        || ((method == Method::PATCH || method == Method::DELETE) && route.starts_with("/api/projects/"))
}

async fn request_health(profile: &BackendProfile, token: Option<&str>) -> Result<BackendHealth, String> {
    let url = api_url(profile, "/api/health")?;
    let client = http_client()?;
    let mut builder = client.get(url).header("X-Akmux-Protocol", PROTOCOL_MAJOR);
    if profile.kind == "remote" {
        let token = token.ok_or_else(|| "Remote profile requires a device credential".to_string())?;
        builder = builder.bearer_auth(token);
    }
    let response = builder.send().await.map_err(safe_network_error)?;
    if !response.status().is_success() {
        return Err(format!("Backend rejected the connection (HTTP {})", response.status().as_u16()));
    }
    let value = response.json::<serde_json::Value>().await.map_err(safe_network_error)?;
    let instance_id = value
        .get("instance_id")
        .and_then(|value| value.as_str())
        .ok_or_else(|| "Backend identity is missing".to_string())?;
    let api_protocol = value
        .get("api_protocol")
        .and_then(|value| value.as_str())
        .ok_or_else(|| "Backend protocol is missing".to_string())?;
    if api_protocol.split('.').next() != Some(PROTOCOL_MAJOR) {
        return Err(format!("Backend protocol {api_protocol} is incompatible"));
    }
    let capabilities = value
        .get("capabilities")
        .and_then(|value| value.as_array())
        .map(|items| items.iter().filter_map(|item| item.as_str().map(str::to_owned)).collect())
        .unwrap_or_default();
    Ok(BackendHealth {
        instance_id: instance_id.into(),
        api_protocol: api_protocol.into(),
        capabilities,
    })
}

fn validate_profile(profile: &BackendProfile) -> Result<(), String> {
    if profile.id.is_empty()
        || profile.id.len() > 80
        || !profile
            .id
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || character == '-' || character == '_')
        || profile.name.trim().is_empty()
        || profile.name.chars().count() > 80
    {
        return Err("Backend profile name and ID are invalid".into());
    }
    let url = Url::parse(&profile.address).map_err(|_| "Backend address is invalid".to_string())?;
    if url.path() != "/" || url.query().is_some() || url.fragment().is_some() || !url.username().is_empty() || url.password().is_some() {
        return Err("Backend address cannot contain credentials, path, query, or fragment".into());
    }
    match profile.kind.as_str() {
        "local" => {
            let loopback = match url.host() {
                Some(url::Host::Domain(host)) => host.eq_ignore_ascii_case("localhost"),
                Some(url::Host::Ipv4(address)) => address.is_loopback(),
                Some(url::Host::Ipv6(address)) => address.is_loopback(),
                None => false,
            };
            if url.scheme() != "http" || !loopback {
                return Err("Local profiles require an HTTP loopback address".into());
            }
            if profile.id != LOCAL_PROFILE_ID {
                return Err("Only the built-in Local profile may use a local backend".into());
            }
            if profile.name != "Local" {
                return Err("The built-in Local profile name cannot be changed".into());
            }
        }
        "remote" if profile.id == LOCAL_PROFILE_ID => return Err("The built-in Local profile cannot become Remote".into()),
        "remote" if url.scheme() == "https" => {}
        "remote" => return Err("Remote profiles require HTTPS".into()),
        _ => return Err("Backend profile type is invalid".into()),
    }
    Ok(())
}

fn validate_profile_identity(state: &BackendProfileState, profile: &BackendProfile) -> Result<(), String> {
    let name = profile.name.trim();
    if state
        .profiles
        .iter()
        .any(|existing| existing.id != profile.id && existing.name.trim().eq_ignore_ascii_case(name))
    {
        return Err("Backend profile names must be unique".into());
    }
    Ok(())
}

fn validate_remote_capabilities(capabilities: &[String]) -> Result<(), String> {
    let missing = REQUIRED_REMOTE_CAPABILITIES
        .iter()
        .filter(|required| !capabilities.iter().any(|capability| capability == **required))
        .copied()
        .collect::<Vec<_>>();
    if missing.is_empty() {
        Ok(())
    } else {
        Err(format!("Backend is missing required security capabilities: {}", missing.join(", ")))
    }
}

fn credential_for_unchanged_profile(state: &BackendProfileState, candidate: &BackendProfile) -> Option<Zeroizing<String>> {
    credential_for_unchanged_profile_with_store(state, candidate, &PlatformCredentialStore)
}

fn credential_for_unchanged_profile_with_store(state: &BackendProfileState, candidate: &BackendProfile, credentials: &impl CredentialStore) -> Option<Zeroizing<String>> {
    state
        .profiles
        .iter()
        .find(|profile| profile.id == candidate.id && profile.kind == "remote" && profile.address == candidate.address && profile.instance_id == candidate.instance_id)
        .and_then(|_| credentials.load(&candidate.id).ok())
}

fn api_url(profile: &BackendProfile, path: &str) -> Result<Url, String> {
    let base = Url::parse(&profile.address).map_err(|_| "Backend address is invalid".to_string())?;
    base.join(path.trim_start_matches('/')).map_err(|_| "Backend API URL is invalid".to_string())
}

fn http_client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(4))
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|_| "Unable to initialize backend networking".to_string())
}

fn safe_network_error(_: reqwest::Error) -> String {
    "Unable to connect to the backend".into()
}

fn state_path(app: &AppHandle) -> Result<PathBuf, String> {
    let directory = app
        .path()
        .app_config_dir()
        .map_err(|_| "Unable to locate the application configuration directory".to_string())?;
    std::fs::create_dir_all(&directory).map_err(|_| "Unable to create the application configuration directory".to_string())?;
    Ok(directory.join("backends.json"))
}

fn load_state(app: &AppHandle) -> Result<BackendProfileState, String> {
    let path = state_path(app)?;
    if !path.exists() {
        return Ok(BackendProfileState::default());
    }
    let bytes = std::fs::read(path).map_err(|_| "Unable to read backend profiles".to_string())?;
    let persisted: PersistedBackendProfileState = serde_json::from_slice(&bytes).map_err(|_| "Backend profiles are invalid".to_string())?;
    let mut state = BackendProfileState {
        profiles: persisted
            .profiles
            .into_iter()
            .map(|profile| profile_from_persisted(profile, &PlatformCredentialStore))
            .collect(),
        active_profile_id: persisted.active_profile_id,
    };
    if !state.profiles.iter().any(|profile| profile.id == LOCAL_PROFILE_ID) {
        state
            .profiles
            .insert(0, BackendProfileState::default().profiles.into_iter().next().expect("default Local profile"));
    }
    for (index, profile) in state.profiles.iter().enumerate() {
        validate_profile(profile).map_err(|_| "Backend profiles are invalid".to_string())?;
        if state.profiles[..index]
            .iter()
            .any(|existing| existing.name.trim().eq_ignore_ascii_case(profile.name.trim()))
        {
            return Err("Backend profiles are invalid".into());
        }
    }
    if !state.profiles.iter().any(|profile| profile.id == state.active_profile_id) {
        state.active_profile_id = LOCAL_PROFILE_ID.into();
    }
    Ok(state)
}

fn profile_from_persisted(profile: PersistedBackendProfile, credentials: &impl CredentialStore) -> BackendProfile {
    let has_credential = profile.kind == "remote" && credentials.load(&profile.id).is_ok();
    BackendProfile {
        requires_auth: profile.kind == "remote" && !has_credential,
        has_credential,
        id: profile.id,
        name: profile.name,
        kind: profile.kind,
        address: profile.address,
        instance_id: profile.instance_id,
        capabilities: profile.capabilities,
    }
}

fn save_state(app: &AppHandle, state: &BackendProfileState) -> Result<(), String> {
    let persisted = PersistedBackendProfileState {
        profiles: state.profiles.iter().map(PersistedBackendProfile::from).collect(),
        active_profile_id: state.active_profile_id.clone(),
    };
    let bytes = serde_json::to_vec_pretty(&persisted).map_err(|_| "Unable to serialize backend profiles".to_string())?;
    save_state_atomically(&state_path(app)?, &bytes)
}

fn save_state_atomically(path: &std::path::Path, bytes: &[u8]) -> Result<(), String> {
    let temporary = path.with_extension(format!("json.{}.tmp", next_operation_id()));
    let result = (|| {
        let mut file = std::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temporary)
            .map_err(|_| "Unable to create a temporary Backend Profile state".to_string())?;
        file.write_all(bytes).map_err(|_| "Unable to write Backend Profile state".to_string())?;
        file.sync_all().map_err(|_| "Unable to synchronize Backend Profile state".to_string())?;
        replace_state_file(&temporary, path).map_err(|_| "Unable to replace Backend Profile state".to_string())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result
}

fn next_operation_id() -> String {
    static SEQUENCE: AtomicU64 = AtomicU64::new(0);
    let sequence = SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let timestamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_nanos();
    format!("{timestamp:032x}{sequence:016x}")
}

fn replace_state_file(temporary: &std::path::Path, destination: &std::path::Path) -> std::io::Result<()> {
    std::fs::rename(temporary, destination)
}

async fn revoke_remote_device(profile: &BackendProfile) -> Result<(), String> {
    let token = load_credential(&profile.id)?;
    let response = http_client()?
        .delete(api_url(profile, "/api/device")?)
        .header("X-Akmux-Protocol", PROTOCOL_MAJOR)
        .bearer_auth(token.as_str())
        .send()
        .await
        .map_err(safe_network_error)?;
    if response.status().is_success() {
        Ok(())
    } else {
        Err("Backend did not confirm device revocation".into())
    }
}

fn temporary_credentials() -> &'static Mutex<HashMap<String, Zeroizing<String>>> {
    static CREDENTIALS: OnceLock<Mutex<HashMap<String, Zeroizing<String>>>> = OnceLock::new();
    CREDENTIALS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn store_temporary_credential(profile_id: &str, token: &str) -> Result<(), String> {
    temporary_credentials()
        .lock()
        .map_err(|_| "Credential memory is unavailable".to_string())?
        .insert(profile_id.into(), Zeroizing::new(token.into()));
    Ok(())
}

fn load_temporary_credential(profile_id: &str) -> Result<Zeroizing<String>, String> {
    temporary_credentials()
        .lock()
        .map_err(|_| "Credential memory is unavailable".to_string())?
        .get(profile_id)
        .cloned()
        .ok_or_else(|| "Backend credential is unavailable; re-authentication is required".into())
}

fn delete_temporary_credential(profile_id: &str) -> Result<(), String> {
    temporary_credentials()
        .lock()
        .map_err(|_| "Credential memory is unavailable".to_string())?
        .remove(profile_id);
    Ok(())
}

#[cfg(not(target_os = "windows"))]
fn platform_store_credential(profile_id: &str, token: &str) -> Result<(), String> {
    store_temporary_credential(profile_id, token)
}

#[cfg(not(target_os = "windows"))]
fn platform_load_credential(profile_id: &str) -> Result<Zeroizing<String>, String> {
    load_temporary_credential(profile_id)
}

#[cfg(not(target_os = "windows"))]
fn platform_delete_credential(profile_id: &str) -> Result<(), String> {
    delete_temporary_credential(profile_id)
}

#[cfg(target_os = "windows")]
fn credential_target(profile_id: &str) -> String {
    format!("AkironMux/backend/{profile_id}")
}

#[cfg(target_os = "windows")]
fn platform_store_credential(profile_id: &str, token: &str) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Security::Credentials::{CredWriteW, CREDENTIALW, CRED_PERSIST_LOCAL_MACHINE, CRED_TYPE_GENERIC};
    let target: Vec<u16> = std::ffi::OsStr::new(&credential_target(profile_id)).encode_wide().chain(Some(0)).collect();
    let mut bytes = token.as_bytes().to_vec();
    let credential = CREDENTIALW {
        Flags: 0,
        Type: CRED_TYPE_GENERIC,
        TargetName: target.as_ptr() as *mut _,
        Comment: std::ptr::null_mut(),
        LastWritten: Default::default(),
        CredentialBlobSize: bytes.len() as u32,
        CredentialBlob: bytes.as_mut_ptr(),
        Persist: CRED_PERSIST_LOCAL_MACHINE,
        AttributeCount: 0,
        Attributes: std::ptr::null_mut(),
        TargetAlias: std::ptr::null_mut(),
        UserName: std::ptr::null_mut(),
    };
    let success = unsafe { CredWriteW(&credential, 0) };
    bytes.fill(0);
    if success != 0 {
        let _ = delete_temporary_credential(profile_id);
        Ok(())
    } else {
        store_temporary_credential(profile_id, token)
    }
}

#[cfg(target_os = "windows")]
fn platform_load_credential(profile_id: &str) -> Result<Zeroizing<String>, String> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Security::Credentials::{CredFree, CredReadW, CREDENTIALW, CRED_TYPE_GENERIC};
    if let Ok(token) = load_temporary_credential(profile_id) {
        return Ok(token);
    }
    let target: Vec<u16> = std::ffi::OsStr::new(&credential_target(profile_id)).encode_wide().chain(Some(0)).collect();
    let mut pointer: *mut CREDENTIALW = std::ptr::null_mut();
    if unsafe { CredReadW(target.as_ptr(), CRED_TYPE_GENERIC, 0, &mut pointer) } == 0 || pointer.is_null() {
        return load_temporary_credential(profile_id);
    }
    let credential = unsafe { &*pointer };
    let size = credential.CredentialBlobSize as usize;
    if credential.CredentialBlob.is_null() || size == 0 || size > 16 * 1024 {
        unsafe { CredFree(pointer.cast()) };
        return Err("Backend credential is invalid".into());
    }
    let bytes = unsafe { std::slice::from_raw_parts(credential.CredentialBlob, size) };
    let owned = bytes.to_vec();
    unsafe { CredFree(pointer.cast()) };
    match String::from_utf8(owned) {
        Ok(token) => Ok(Zeroizing::new(token)),
        Err(error) => {
            let mut bytes = error.into_bytes();
            bytes.fill(0);
            Err("Backend credential is invalid".into())
        }
    }
}

#[cfg(target_os = "windows")]
fn platform_delete_credential(profile_id: &str) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Foundation::GetLastError;
    use windows_sys::Win32::Security::Credentials::{CredDeleteW, CRED_TYPE_GENERIC};
    let target: Vec<u16> = std::ffi::OsStr::new(&credential_target(profile_id)).encode_wide().chain(Some(0)).collect();
    let success = unsafe { CredDeleteW(target.as_ptr(), CRED_TYPE_GENERIC, 0) };
    let error = if success == 0 { unsafe { GetLastError() } } else { 0 };
    delete_temporary_credential(profile_id)?;
    if success != 0 || error == 1168 {
        Ok(())
    } else {
        Err("Windows Credential Manager could not delete the credential".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct FakeCredentialStore(Mutex<HashMap<String, String>>);

    impl CredentialStore for FakeCredentialStore {
        fn store(&self, profile_id: &str, token: &str) -> Result<(), String> {
            self.0.lock().unwrap().insert(profile_id.into(), token.into());
            Ok(())
        }

        fn load(&self, profile_id: &str) -> Result<Zeroizing<String>, String> {
            self.0
                .lock()
                .unwrap()
                .get(profile_id)
                .cloned()
                .map(Zeroizing::new)
                .ok_or_else(|| "missing credential".into())
        }

        fn delete(&self, profile_id: &str) -> Result<(), String> {
            self.0.lock().unwrap().remove(profile_id);
            Ok(())
        }
    }

    fn profile(kind: &str, address: &str) -> BackendProfile {
        BackendProfile {
            id: if kind == "local" { LOCAL_PROFILE_ID.into() } else { "test".into() },
            name: if kind == "local" { "Local".into() } else { "Test".into() },
            kind: kind.into(),
            address: address.into(),
            instance_id: None,
            has_credential: false,
            requires_auth: kind == "remote",
            capabilities: Vec::new(),
        }
    }

    #[test]
    fn local_profiles_are_restricted_to_http_loopback() {
        assert!(validate_profile(&profile("local", "http://127.0.0.1:17321")).is_ok());
        assert!(validate_profile(&profile("local", "http://localhost:17321")).is_ok());
        assert!(validate_profile(&profile("local", "http://[::1]:17321")).is_ok());
        assert!(validate_profile(&profile("local", "http://192.168.1.2:17321")).is_err());
        assert!(validate_profile(&profile("local", "https://127.0.0.1:17321")).is_err());
    }

    #[test]
    fn remote_profiles_require_clean_https_origins() {
        assert!(validate_profile(&profile("remote", "https://mux.example.com")).is_ok());
        assert!(validate_profile(&profile("remote", "http://mux.example.com")).is_err());
        assert!(validate_profile(&profile("remote", "https://mux.example.com/path")).is_err());
    }

    #[test]
    fn serialized_profile_state_contains_no_credential_material() {
        let state = PersistedBackendProfileState {
            profiles: BackendProfileState::default().profiles.iter().map(PersistedBackendProfile::from).collect(),
            active_profile_id: LOCAL_PROFILE_ID.into(),
        };
        let serialized = serde_json::to_string(&state).unwrap();
        assert!(!serialized.contains("akmux_1_"));
        assert!(!serialized.contains("token"));
        assert!(!serialized.contains("hasCredential"));
    }

    #[test]
    fn restored_remote_profiles_without_native_credentials_require_authentication() {
        let profile = PersistedBackendProfile {
            id: "restored".into(),
            name: "Restored".into(),
            kind: "remote".into(),
            address: "https://mux.example.com".into(),
            instance_id: Some("instance-a".into()),
            capabilities: REQUIRED_REMOTE_CAPABILITIES.iter().map(|value| (*value).into()).collect(),
        };
        let restored = profile_from_persisted(profile, &FakeCredentialStore::default());
        assert!(restored.requires_auth);
        assert!(!restored.has_credential);
    }

    #[test]
    fn profile_names_are_unique_and_local_identity_is_fixed() {
        let state = BackendProfileState::default();
        let duplicate = BackendProfile {
            id: "remote-1".into(),
            name: " local ".into(),
            kind: "remote".into(),
            address: "https://mux.example.com".into(),
            instance_id: None,
            has_credential: false,
            requires_auth: true,
            capabilities: Vec::new(),
        };
        assert!(validate_profile_identity(&state, &duplicate).is_err());

        let mut changed_local = profile("remote", "https://mux.example.com");
        changed_local.id = LOCAL_PROFILE_ID.into();
        assert!(validate_profile(&changed_local).is_err());
    }

    #[test]
    fn stored_credentials_are_bound_to_the_saved_origin_and_identity() {
        let credentials = FakeCredentialStore::default();
        credentials.store("remote-1", "secret").unwrap();
        let saved = BackendProfile {
            id: "remote-1".into(),
            name: "Remote".into(),
            kind: "remote".into(),
            address: "https://mux.example.com".into(),
            instance_id: Some("instance-a".into()),
            has_credential: true,
            requires_auth: false,
            capabilities: REQUIRED_REMOTE_CAPABILITIES.iter().map(|value| (*value).into()).collect(),
        };
        let state = BackendProfileState {
            profiles: vec![BackendProfileState::default().profiles.into_iter().next().unwrap(), saved.clone()],
            active_profile_id: "remote-1".into(),
        };
        assert_eq!(
            credential_for_unchanged_profile_with_store(&state, &saved, &credentials).as_deref().map(String::as_str),
            Some("secret")
        );
        assert!(credential_for_unchanged_profile_with_store(
            &state,
            &BackendProfile {
                address: "https://attacker.example.com".into(),
                ..saved.clone()
            },
            &credentials,
        )
        .is_none());
        assert!(credential_for_unchanged_profile_with_store(
            &state,
            &BackendProfile {
                instance_id: Some("instance-b".into()),
                ..saved
            },
            &credentials,
        )
        .is_none());
        credentials.delete("remote-1").unwrap();
    }

    #[test]
    fn webview_transport_is_limited_to_gui_routes() {
        assert!(is_allowed_api_request(&Method::GET, "/api/workspaces?q=codex"));
        assert!(is_allowed_api_request(&Method::POST, "/api/auth/ws-ticket"));
        assert!(is_allowed_api_request(&Method::DELETE, "/api/sessions/session-1"));
        assert!(!is_allowed_api_request(&Method::DELETE, "/api/device"));
        assert!(!is_allowed_api_request(&Method::GET, "/api/sessions/../admin"));
    }

    #[test]
    fn remote_profiles_cannot_downgrade_security_capabilities() {
        let complete = REQUIRED_REMOTE_CAPABILITIES.iter().map(|value| (*value).into()).collect::<Vec<_>>();
        assert!(validate_remote_capabilities(&complete).is_ok());
        assert!(validate_remote_capabilities(&["device-auth".into(), "ws-ticket".into(), "control-lease".into()]).is_ok());
        assert!(validate_remote_capabilities(&["workspace-v1".into()]).is_err());
    }

    #[test]
    fn pairing_links_are_validated_and_bound_to_the_profile_origin() {
        let parsed = parse_pairing_link(
            "akmux://pair?url=https%3A%2F%2Fmux.example.com&name=Office&code=short-lived-code",
            "https://mux.example.com",
        )
        .unwrap();
        assert_eq!(parsed.code, "short-lived-code");
        assert!(parse_pairing_link(
            "akmux://pair?url=https%3A%2F%2Fevil.example.com&name=Office&code=short-lived-code",
            "https://mux.example.com",
        )
        .is_err());
    }

    #[test]
    fn backend_profile_state_is_replaced_without_leaving_partial_files() {
        let directory = std::env::temp_dir().join(format!("akmux-backend-state-test-{}-{}", std::process::id(), next_operation_id()));
        std::fs::create_dir(&directory).unwrap();
        let path = directory.join("backends.json");
        std::fs::write(&path, b"old").unwrap();

        save_state_atomically(&path, b"new profile state").unwrap();

        assert_eq!(std::fs::read(&path).unwrap(), b"new profile state");
        assert_eq!(std::fs::read_dir(&directory).unwrap().count(), 1);
        std::fs::remove_file(path).unwrap();
        std::fs::remove_dir(directory).unwrap();
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn windows_credential_adapter_round_trips_without_plaintext_profile_storage() {
        let profile_id = format!("credential-test-{}", std::process::id());
        let token = "akmux_1_test-only-device-credential";
        platform_store_credential(&profile_id, token).unwrap();
        let loaded = platform_load_credential(&profile_id).unwrap();
        assert_eq!(loaded.as_str(), token);
        platform_delete_credential(&profile_id).unwrap();
        assert!(platform_load_credential(&profile_id).is_err());
    }
}
