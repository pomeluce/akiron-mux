use std::{
    collections::HashMap,
    fs::OpenOptions,
    io::Write,
    net::{IpAddr, SocketAddr},
    path::Path,
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use hmac::{Hmac, Mac};
use rand::{rngs::OsRng, RngCore};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use subtle::ConstantTimeEq;
use tokio::sync::broadcast;
use url::Url;

use crate::db::{remote::BackendDevice, Db};

const TOKEN_PREFIX: &str = "akmux_1";
const TICKET_TTL_MS: i64 = 30_000;
const PAIRING_TTL_MS: i64 = 60_000;
const LEASE_RECOVERY_TTL_MS: i64 = 30_000;
type HmacSha256 = Hmac<Sha256>;

pub const DEFAULT_REMOTE_BIND: &str = "127.0.0.1:17322";

#[derive(Debug, Clone)]
pub struct RemoteBackendConfig {
    pub enabled: bool,
    pub bind: SocketAddr,
    pub public_url: Option<Url>,
    pub allow_wildcard_bind: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteListenerConfig {
    pub bind: SocketAddr,
    pub public_url: Url,
}

impl RemoteBackendConfig {
    pub fn load(db: &Db) -> anyhow::Result<Self> {
        let bind = db.get_setting("remote.bind").unwrap_or_else(|| DEFAULT_REMOTE_BIND.to_owned()).parse::<SocketAddr>()?;
        let public_url = db.get_setting("remote.public_url").map(|value| validate_public_url(&value)).transpose()?;
        Ok(Self {
            enabled: db.get_setting("remote.enabled").as_deref() == Some("true"),
            bind,
            public_url,
            allow_wildcard_bind: db.get_setting("remote.allow_wildcard_bind").as_deref() == Some("true"),
        })
    }

    pub fn listener(&self, db: &Db) -> anyhow::Result<Option<RemoteListenerConfig>> {
        if !self.enabled {
            return Ok(None);
        }
        anyhow::ensure!(db.has_active_backend_device()?, "Remote backend requires at least one active device credential");
        validate_bind(self.bind.ip(), self.allow_wildcard_bind)?;
        let public_url = self.public_url.clone().ok_or_else(|| anyhow::anyhow!("Remote public URL is required"))?;
        Ok(Some(RemoteListenerConfig { bind: self.bind, public_url }))
    }
}

pub fn validate_public_url(value: &str) -> anyhow::Result<Url> {
    let url = Url::parse(value)?;
    anyhow::ensure!(url.scheme() == "https", "Remote public URL must use HTTPS");
    anyhow::ensure!(url.username().is_empty() && url.password().is_none(), "Remote public URL cannot contain user information");
    anyhow::ensure!(
        url.path() == "/" && url.query().is_none() && url.fragment().is_none(),
        "Remote public URL cannot contain a path, query, or fragment"
    );
    Ok(url)
}

pub fn validate_bind(ip: IpAddr, allow_wildcard_bind: bool) -> anyhow::Result<()> {
    anyhow::ensure!(
        is_safe_bind(ip) || (ip.is_unspecified() && allow_wildcard_bind),
        if ip.is_unspecified() {
            "Wildcard Remote bind requires --allow-wildcard-bind and firewall or TLS-proxy safeguards"
        } else {
            "Remote bind must be loopback, private LAN, or Tailnet; public addresses are rejected"
        }
    );
    Ok(())
}

pub fn is_safe_bind(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            let octets = ip.octets();
            ip.is_loopback() || ip.is_private() || ip.is_link_local() || (octets[0] == 100 && (64..=127).contains(&octets[1]))
        }
        IpAddr::V6(ip) => {
            let octets = ip.octets();
            let unique_local = octets[0] & 0xfe == 0xfc;
            let unicast_link_local = octets[0] == 0xfe && octets[1] & 0xc0 == 0x80;
            ip.is_loopback() || unique_local || unicast_link_local
        }
    }
}

#[derive(Debug, Clone)]
pub struct DeviceIdentity {
    pub token_id: String,
    pub name: String,
}

#[derive(Debug, Clone)]
struct Ticket {
    device: DeviceIdentity,
    session_id: String,
    expires_at_ms: i64,
}

#[derive(Debug, Clone, Copy)]
struct RateLimitEntry {
    failures: u8,
    blocked_until_ms: i64,
}

#[derive(Debug, Clone)]
struct PendingPairing {
    code: String,
    expires_at_ms: i64,
    device_name: Option<String>,
    source: Option<String>,
    approved: bool,
    notify: Arc<tokio::sync::Notify>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PairingOffer {
    pub id: String,
    pub code: String,
    pub deep_link: String,
    pub expires_at_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingPairingInfo {
    pub id: String,
    pub device_name: Option<String>,
    pub source: Option<String>,
    pub expires_at_ms: i64,
}

pub struct PairingClaim {
    id: String,
    expires_at_ms: i64,
    notify: Arc<tokio::sync::Notify>,
}

impl PairingClaim {
    pub fn id(&self) -> &str {
        &self.id
    }
}

#[derive(Clone)]
pub struct RemoteSecurity {
    pepper: Arc<[u8; 32]>,
    tickets: Arc<Mutex<HashMap<String, Ticket>>>,
    pairings: Arc<Mutex<HashMap<String, PendingPairing>>>,
    rate_limits: Arc<Mutex<HashMap<String, RateLimitEntry>>>,
    revocations: broadcast::Sender<String>,
}

impl RemoteSecurity {
    pub fn ephemeral() -> Self {
        let mut pepper = [0_u8; 32];
        OsRng.fill_bytes(&mut pepper);
        let (revocations, _) = broadcast::channel(64);
        Self {
            pepper: Arc::new(pepper),
            tickets: Arc::new(Mutex::new(HashMap::new())),
            pairings: Arc::new(Mutex::new(HashMap::new())),
            rate_limits: Arc::new(Mutex::new(HashMap::new())),
            revocations,
        }
    }

    pub fn load_or_create(path: &Path) -> anyhow::Result<Self> {
        let pepper = if path.exists() {
            let bytes = std::fs::read(path)?;
            anyhow::ensure!(bytes.len() == 32, "Remote credential pepper has an invalid length");
            let mut pepper = [0_u8; 32];
            pepper.copy_from_slice(&bytes);
            pepper
        } else {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let mut pepper = [0_u8; 32];
            OsRng.fill_bytes(&mut pepper);
            let mut options = OpenOptions::new();
            options.create_new(true).write(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            let mut file = options.open(path)?;
            file.write_all(&pepper)?;
            file.sync_all()?;
            pepper
        };
        let (revocations, _) = broadcast::channel(64);
        Ok(Self {
            pepper: Arc::new(pepper),
            tickets: Arc::new(Mutex::new(HashMap::new())),
            pairings: Arc::new(Mutex::new(HashMap::new())),
            rate_limits: Arc::new(Mutex::new(HashMap::new())),
            revocations,
        })
    }

    #[cfg(test)]
    fn for_tests() -> Self {
        Self::ephemeral()
    }

    fn digest(&self, token_id: &str, secret: &[u8]) -> [u8; 32] {
        let mut mac = HmacSha256::new_from_slice(self.pepper.as_ref()).expect("HMAC accepts a 256-bit key");
        mac.update(token_id.as_bytes());
        mac.update(&[0]);
        mac.update(secret);
        mac.finalize().into_bytes().into()
    }

    pub fn create_device(&self, db: &Db, name: &str) -> anyhow::Result<(BackendDevice, String)> {
        let name = name.trim();
        anyhow::ensure!(!name.is_empty() && name.chars().count() <= 80, "Device name must contain 1 to 80 characters");
        let mut id_bytes = [0_u8; 12];
        let mut secret = [0_u8; 32];
        OsRng.fill_bytes(&mut id_bytes);
        OsRng.fill_bytes(&mut secret);
        let token_id = URL_SAFE_NO_PAD.encode(id_bytes);
        let digest = self.digest(&token_id, &secret);
        let device = BackendDevice {
            token_id: token_id.clone(),
            name: name.to_owned(),
            created_at_ms: now_ms(),
            last_used_at_ms: None,
            revoked_at_ms: None,
        };
        db.insert_backend_device(&device, &digest)?;
        db.record_backend_audit("device.created", Some(&token_id), None, now_ms())?;
        let token = format!("{TOKEN_PREFIX}_{token_id}_{}", URL_SAFE_NO_PAD.encode(secret));
        Ok((device, token))
    }

    pub fn authenticate(&self, db: &Db, token: &str) -> anyhow::Result<Option<DeviceIdentity>> {
        let Some((token_id, secret)) = parse_token(token) else {
            return Ok(None);
        };
        let Some((device, expected)) = db.backend_device_digest(token_id)? else {
            return Ok(None);
        };
        if device.revoked_at_ms.is_some() || expected.len() != 32 {
            return Ok(None);
        }
        let actual = self.digest(token_id, &secret);
        if actual.ct_eq(expected.as_slice()).unwrap_u8() != 1 {
            return Ok(None);
        }
        db.touch_backend_device(token_id, now_ms())?;
        Ok(Some(DeviceIdentity {
            token_id: device.token_id,
            name: device.name,
        }))
    }

    pub fn rate_limit_allows(&self, key: &str) -> bool {
        self.rate_limits
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(key)
            .map_or(true, |entry| entry.blocked_until_ms <= now_ms())
    }

    pub fn record_auth_failure(&self, key: &str) {
        self.record_rate_failure(key, 1);
    }

    fn record_rate_failure(&self, key: &str, block_after: u8) {
        let mut limits = self.rate_limits.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let entry = limits.entry(key.to_owned()).or_insert(RateLimitEntry { failures: 0, blocked_until_ms: 0 });
        entry.failures = entry.failures.saturating_add(1);
        if entry.failures >= block_after {
            let exponent = entry.failures.saturating_sub(block_after).min(7) as u32;
            entry.blocked_until_ms = now_ms() + 250_i64.saturating_mul(2_i64.pow(exponent));
        }
    }

    pub fn clear_auth_failures(&self, key: &str) {
        self.rate_limits.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).remove(key);
    }

    pub fn authentication_failure_rate_limit_allows(&self, source: &str, token_id: &str) -> bool {
        self.rate_limit_allows(&format!("auth-source:{source}")) && self.rate_limit_allows(&format!("auth-token:{token_id}"))
    }

    pub fn record_authentication_failure(&self, source: &str, token_id: &str) {
        // A reverse proxy is intentionally treated as one untrusted source because AkironMux
        // does not trust forwarding headers. Keep a burst allowance so one hostile client
        // cannot immediately lock every legitimate client behind that proxy.
        self.record_rate_failure(&format!("auth-source:{source}"), 20);
        self.record_auth_failure(&format!("auth-token:{token_id}"));
    }

    pub fn clear_authentication_token_failures(&self, token_id: &str) {
        self.clear_auth_failures(&format!("auth-token:{token_id}"));
    }

    pub fn pairing_failure_rate_limit_allows(&self, source: &str, code: &str) -> bool {
        let source_allowed = self.rate_limit_allows(&format!("pair-source:{source}"));
        match self.pairing_id_for_code(code) {
            Some(pairing_id) => source_allowed && self.rate_limit_allows(&format!("pair-id:{pairing_id}")),
            None => source_allowed,
        }
    }

    pub fn record_pairing_failure(&self, source: &str, code: &str) {
        self.record_rate_failure(&format!("pair-source:{source}"), 10);
        if let Some(pairing_id) = self.pairing_id_for_code(code) {
            self.record_auth_failure(&format!("pair-id:{pairing_id}"));
        }
    }

    pub fn clear_pairing_id_failures(&self, pairing_id: &str) {
        self.clear_auth_failures(&format!("pair-id:{pairing_id}"));
    }

    pub fn pairing_code_is_pending(&self, code: &str) -> bool {
        self.pairing_id_for_code(code).is_some()
    }

    fn pairing_id_for_code(&self, code: &str) -> Option<String> {
        self.pairings
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .iter()
            .find(|(_, pairing)| pairing.code.as_bytes().ct_eq(code.as_bytes()).unwrap_u8() == 1)
            .map(|(id, _)| id.clone())
    }

    pub fn revoke_device(&self, db: &Db, token_id: &str) -> anyhow::Result<bool> {
        let now = now_ms();
        let revoked = db.revoke_backend_device(token_id, now)?;
        if revoked {
            db.record_backend_audit("device.revoked", Some(token_id), Some("local-api"), now)?;
            let _ = self.revocations.send(token_id.to_owned());
        }
        Ok(revoked)
    }

    pub fn subscribe_revocations(&self) -> broadcast::Receiver<String> {
        self.revocations.subscribe()
    }

    pub fn issue_ticket(&self, device: DeviceIdentity, session_id: String) -> String {
        let mut bytes = [0_u8; 32];
        OsRng.fill_bytes(&mut bytes);
        let value = URL_SAFE_NO_PAD.encode(bytes);
        let mut tickets = self.tickets.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let now = now_ms();
        tickets.retain(|_, ticket| ticket.expires_at_ms > now);
        tickets.insert(
            value.clone(),
            Ticket {
                device,
                session_id,
                expires_at_ms: now + TICKET_TTL_MS,
            },
        );
        value
    }

    pub fn consume_ticket(&self, value: &str, session_id: &str) -> Option<DeviceIdentity> {
        let ticket = self.tickets.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).remove(value)?;
        (ticket.expires_at_ms > now_ms() && ticket.session_id == session_id).then_some(ticket.device)
    }

    pub fn create_pairing(&self, public_url: &str, backend_name: &str) -> anyhow::Result<PairingOffer> {
        let url = url::Url::parse(public_url)?;
        anyhow::ensure!(url.scheme() == "https", "Pairing URL must use HTTPS");
        let id = generate_opaque_id(18);
        let code = generate_opaque_id(18);
        let expires_at_ms = now_ms() + PAIRING_TTL_MS;
        let query = url::form_urlencoded::Serializer::new(String::new())
            .append_pair("url", public_url)
            .append_pair("name", backend_name)
            .append_pair("code", &code)
            .finish();
        let offer = PairingOffer {
            id: id.clone(),
            code: code.clone(),
            deep_link: format!("akmux://pair?{query}"),
            expires_at_ms,
        };
        let mut pairings = self.pairings.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        pairings.retain(|_, pairing| pairing.expires_at_ms > now_ms());
        pairings.insert(
            id,
            PendingPairing {
                code,
                expires_at_ms,
                device_name: None,
                source: None,
                approved: false,
                notify: Arc::new(tokio::sync::Notify::new()),
            },
        );
        Ok(offer)
    }

    pub fn begin_pairing_claim(&self, code: &str, device_name: &str, source: &str) -> anyhow::Result<PairingClaim> {
        let device_name = device_name.trim();
        anyhow::ensure!(!device_name.is_empty() && device_name.chars().count() <= 80, "Device name must contain 1 to 80 characters");
        let mut pairings = self.pairings.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let now = now_ms();
        pairings.retain(|_, pairing| pairing.expires_at_ms > now);
        let (id, pairing) = pairings
            .iter_mut()
            .find(|(_, pairing)| pairing.code.as_bytes().ct_eq(code.as_bytes()).unwrap_u8() == 1)
            .ok_or_else(|| anyhow::anyhow!("Pairing code is invalid or expired"))?;
        anyhow::ensure!(pairing.device_name.is_none(), "Pairing code has already been claimed");
        pairing.device_name = Some(device_name.to_owned());
        pairing.source = Some(source.to_owned());
        Ok(PairingClaim {
            id: id.clone(),
            expires_at_ms: pairing.expires_at_ms,
            notify: Arc::clone(&pairing.notify),
        })
    }

    pub fn pending_pairings(&self) -> Vec<PendingPairingInfo> {
        let mut pairings = self.pairings.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        pairings.retain(|_, pairing| pairing.expires_at_ms > now_ms());
        pairings
            .iter()
            .map(|(id, pairing)| PendingPairingInfo {
                id: id.clone(),
                device_name: pairing.device_name.clone(),
                source: pairing.source.clone(),
                expires_at_ms: pairing.expires_at_ms,
            })
            .collect()
    }

    pub fn approve_pairing(&self, id: &str) -> anyhow::Result<()> {
        let mut pairings = self.pairings.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let pairing = pairings.get_mut(id).ok_or_else(|| anyhow::anyhow!("Pairing request is missing or expired"))?;
        anyhow::ensure!(pairing.expires_at_ms > now_ms(), "Pairing request is expired");
        anyhow::ensure!(pairing.device_name.is_some(), "The remote device has not submitted this pairing request yet");
        pairing.approved = true;
        pairing.notify.notify_one();
        Ok(())
    }

    pub fn cancel_pairing(&self, id: &str) -> bool {
        self.pairings.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).remove(id).is_some()
    }

    pub async fn finish_pairing(&self, claim: PairingClaim) -> anyhow::Result<String> {
        let timeout_ms = (claim.expires_at_ms - now_ms()).max(0) as u64;
        tokio::time::timeout(std::time::Duration::from_millis(timeout_ms), claim.notify.notified())
            .await
            .map_err(|_| anyhow::anyhow!("Pairing confirmation expired"))?;
        let device_name = {
            let mut pairings = self.pairings.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            let pairing = pairings.remove(&claim.id).ok_or_else(|| anyhow::anyhow!("Pairing request is no longer available"))?;
            anyhow::ensure!(pairing.approved && pairing.expires_at_ms > now_ms(), "Pairing request was not approved in time");
            pairing.device_name.ok_or_else(|| anyhow::anyhow!("Pairing device name is missing"))?
        };
        Ok(device_name)
    }
}

fn parse_token(token: &str) -> Option<(&str, Vec<u8>)> {
    let rest = token.strip_prefix("akmux_1_")?;
    const TOKEN_ID_LENGTH: usize = 16;
    let token_id = rest.get(..TOKEN_ID_LENGTH)?;
    let secret = rest.get(TOKEN_ID_LENGTH..)?.strip_prefix('_')?;
    let secret = URL_SAFE_NO_PAD.decode(secret).ok()?;
    (secret.len() == 32).then_some((token_id, secret))
}

pub fn token_public_id(token: &str) -> Option<&str> {
    parse_token(token).map(|(id, _)| id)
}

pub fn now_ms() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis() as i64
}

fn generate_opaque_id(byte_count: usize) -> String {
    let mut bytes = vec![0_u8; byte_count];
    OsRng.fill_bytes(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}

pub fn new_instance_id() -> String {
    generate_opaque_id(18)
}

pub fn new_connection_id() -> String {
    generate_opaque_id(18)
}

#[derive(Debug, Clone, Serialize)]
pub struct LeaseState {
    pub version: u64,
    pub controller_connection_id: Option<String>,
    pub controller_device_name: Option<String>,
    pub recovery_expires_at_ms: Option<i64>,
}

#[derive(Debug, Clone)]
pub struct LeaseConnection {
    pub state: LeaseState,
    pub recovery_credential: Option<String>,
}

#[derive(Debug, Clone)]
struct LeaseRecovery {
    credential: String,
    expires_at_ms: Option<i64>,
}

#[derive(Default)]
struct LeaseRegistry {
    states: HashMap<String, LeaseState>,
    recoveries: HashMap<String, LeaseRecovery>,
    events: HashMap<String, broadcast::Sender<LeaseState>>,
}

#[derive(Default, Clone)]
pub struct ControlLeases {
    inner: Arc<Mutex<LeaseRegistry>>,
}

impl ControlLeases {
    pub fn connect(&self, session_id: &str, connection_id: &str, device_name: &str) -> LeaseConnection {
        let mut registry = self.inner.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        Self::expire_recovery(&mut registry, session_id);
        let lease = registry.states.entry(session_id.to_owned()).or_insert(LeaseState {
            version: 0,
            controller_connection_id: None,
            controller_device_name: None,
            recovery_expires_at_ms: None,
        });
        let initial = lease.version == 0 && lease.controller_connection_id.is_none();
        if initial {
            lease.version += 1;
            lease.controller_connection_id = Some(connection_id.to_owned());
            lease.controller_device_name = Some(device_name.to_owned());
            lease.recovery_expires_at_ms = None;
        }
        let state = lease.clone();
        let recovery_credential = initial.then(|| {
            let credential = generate_opaque_id(32);
            registry.recoveries.insert(
                session_id.to_owned(),
                LeaseRecovery {
                    credential: credential.clone(),
                    expires_at_ms: None,
                },
            );
            credential
        });
        Self::broadcast(&mut registry, session_id, &state);
        LeaseConnection { state, recovery_credential }
    }

    pub fn recover_control(&self, session_id: &str, connection_id: &str, device_name: &str, credential: &str) -> LeaseConnection {
        let mut registry = self.inner.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        Self::expire_recovery(&mut registry, session_id);
        let restored = registry
            .recoveries
            .get(session_id)
            .is_some_and(|recovery| recovery.expires_at_ms.is_some() && recovery.credential.as_bytes().ct_eq(credential.as_bytes()).unwrap_u8() == 1);
        let lease = registry.states.entry(session_id.to_owned()).or_insert(LeaseState {
            version: 0,
            controller_connection_id: None,
            controller_device_name: None,
            recovery_expires_at_ms: None,
        });
        let recovery_credential = restored.then(|| {
            lease.version += 1;
            lease.controller_connection_id = Some(connection_id.to_owned());
            lease.controller_device_name = Some(device_name.to_owned());
            lease.recovery_expires_at_ms = None;
            generate_opaque_id(32)
        });
        let state = lease.clone();
        if let Some(credential) = recovery_credential.as_ref() {
            registry.recoveries.insert(
                session_id.to_owned(),
                LeaseRecovery {
                    credential: credential.clone(),
                    expires_at_ms: None,
                },
            );
        }
        Self::broadcast(&mut registry, session_id, &state);
        LeaseConnection { state, recovery_credential }
    }

    pub fn take_control(&self, session_id: &str, connection_id: &str, device_name: &str, expected_version: u64) -> LeaseConnection {
        let mut registry = self.inner.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        Self::expire_recovery(&mut registry, session_id);
        let lease = registry.states.entry(session_id.to_owned()).or_insert(LeaseState {
            version: 0,
            controller_connection_id: None,
            controller_device_name: None,
            recovery_expires_at_ms: None,
        });
        let mut recovery_credential = None;
        if lease.version == expected_version {
            lease.version += 1;
            lease.controller_connection_id = Some(connection_id.to_owned());
            lease.controller_device_name = Some(device_name.to_owned());
            lease.recovery_expires_at_ms = None;
            recovery_credential = Some(generate_opaque_id(32));
        }
        let state = lease.clone();
        if let Some(credential) = recovery_credential.as_ref() {
            registry.recoveries.insert(
                session_id.to_owned(),
                LeaseRecovery {
                    credential: credential.clone(),
                    expires_at_ms: None,
                },
            );
        }
        Self::broadcast(&mut registry, session_id, &state);
        LeaseConnection { state, recovery_credential }
    }

    pub fn state(&self, session_id: &str) -> LeaseState {
        let mut registry = self.inner.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        Self::expire_recovery(&mut registry, session_id);
        registry.states.get(session_id).cloned().unwrap_or(LeaseState {
            version: 0,
            controller_connection_id: None,
            controller_device_name: None,
            recovery_expires_at_ms: None,
        })
    }

    pub fn can_write(&self, session_id: &str, connection_id: &str) -> bool {
        self.state(session_id).controller_connection_id.as_deref() == Some(connection_id)
    }

    pub fn disconnect(&self, session_id: &str, connection_id: &str) {
        let mut registry = self.inner.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut changed = None;
        let mut recovery_expires_at_ms = None;
        if let Some(lease) = registry.states.get_mut(session_id) {
            if lease.controller_connection_id.as_deref() == Some(connection_id) {
                lease.version += 1;
                lease.controller_connection_id = None;
                let expires_at_ms = now_ms() + LEASE_RECOVERY_TTL_MS;
                lease.recovery_expires_at_ms = Some(expires_at_ms);
                recovery_expires_at_ms = Some(expires_at_ms);
                changed = Some(lease.clone());
            }
        }
        if let (Some(recovery), Some(expires_at_ms)) = (registry.recoveries.get_mut(session_id), recovery_expires_at_ms) {
            recovery.expires_at_ms = Some(expires_at_ms);
        }
        if let Some(state) = changed {
            Self::broadcast(&mut registry, session_id, &state);
        }
    }

    pub fn subscribe(&self, session_id: &str) -> broadcast::Receiver<LeaseState> {
        let mut registry = self.inner.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        registry.events.entry(session_id.to_owned()).or_insert_with(|| broadcast::channel(32).0).subscribe()
    }

    fn broadcast(registry: &mut LeaseRegistry, session_id: &str, state: &LeaseState) {
        let sender = registry.events.entry(session_id.to_owned()).or_insert_with(|| broadcast::channel(32).0);
        let _ = sender.send(state.clone());
    }

    fn expire_recovery(registry: &mut LeaseRegistry, session_id: &str) {
        let expired = registry
            .recoveries
            .get(session_id)
            .and_then(|recovery| recovery.expires_at_ms)
            .is_some_and(|expires_at_ms| expires_at_ms <= now_ms());
        if !expired {
            return;
        }
        registry.recoveries.remove(session_id);
        if let Some(lease) = registry.states.get_mut(session_id) {
            lease.version += 1;
            lease.controller_device_name = None;
            lease.recovery_expires_at_ms = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_is_verified_and_plaintext_is_not_persisted() {
        let security = RemoteSecurity::for_tests();
        let directory = tempfile::tempdir().unwrap();
        let database_path = directory.path().join("akmux.db");
        let db = Db::open(&database_path).unwrap();
        let (device, token) = security.create_device(&db, "Android").unwrap();
        assert_eq!(security.authenticate(&db, &token).unwrap().unwrap().name, "Android");
        assert!(security.authenticate(&db, &(token.clone() + "x")).unwrap().is_none());
        let (_, stored) = db.backend_device_digest(&device.token_id).unwrap().unwrap();
        assert!(!String::from_utf8_lossy(&stored).contains("akmux_1_"));
        drop(db);
        assert!(!String::from_utf8_lossy(&std::fs::read(database_path).unwrap()).contains(&token));
    }

    #[tokio::test]
    async fn revocation_invalidates_authentication_and_notifies_active_connections() {
        let security = RemoteSecurity::for_tests();
        let db = Db::open(Path::new(":memory:")).unwrap();
        let (device, token) = security.create_device(&db, "Android").unwrap();
        let mut revocations = security.subscribe_revocations();

        assert!(security.authenticate(&db, &token).unwrap().is_some());
        assert!(security.revoke_device(&db, &device.token_id).unwrap());
        assert_eq!(revocations.recv().await.unwrap(), device.token_id);
        assert!(security.authenticate(&db, &token).unwrap().is_none());
    }

    #[test]
    fn websocket_ticket_is_bound_and_single_use() {
        let security = RemoteSecurity::for_tests();
        let device = DeviceIdentity {
            token_id: "id".into(),
            name: "Desktop".into(),
        };
        let ticket = security.issue_ticket(device, "session-a".into());
        assert!(security.consume_ticket(&ticket, "session-b").is_none());
        assert!(security.consume_ticket(&ticket, "session-a").is_none());
        let ticket = security.issue_ticket(
            DeviceIdentity {
                token_id: "id".into(),
                name: "Desktop".into(),
            },
            "session-a".into(),
        );
        assert_eq!(security.consume_ticket(&ticket, "session-a").unwrap().name, "Desktop");
        assert!(security.consume_ticket(&ticket, "session-a").is_none());
    }

    #[test]
    fn expired_websocket_ticket_is_rejected() {
        let security = RemoteSecurity::for_tests();
        let ticket = security.issue_ticket(
            DeviceIdentity {
                token_id: "id".into(),
                name: "Desktop".into(),
            },
            "session".into(),
        );
        security.tickets.lock().unwrap().get_mut(&ticket).unwrap().expires_at_ms = now_ms() - 1;
        assert!(security.consume_ticket(&ticket, "session").is_none());
    }

    #[test]
    fn takeover_makes_previous_controller_read_only() {
        let leases = ControlLeases::default();
        leases.connect("session", "first", "Desktop");
        assert!(leases.can_write("session", "first"));
        let state = leases.take_control("session", "second", "Phone", 1);
        assert_eq!(state.state.controller_device_name.as_deref(), Some("Phone"));
        assert!(!leases.can_write("session", "first"));
        assert!(leases.can_write("session", "second"));
        let stale = leases.take_control("session", "third", "Tablet", 1);
        assert_eq!(stale.state.controller_device_name.as_deref(), Some("Phone"));
        assert!(!leases.can_write("session", "third"));
    }

    #[test]
    fn authentication_failures_are_rate_limited_without_permanent_lockout() {
        let security = RemoteSecurity::for_tests();
        assert!(security.rate_limit_allows("source:token"));
        security.record_auth_failure("source:token");
        assert!(!security.rate_limit_allows("source:token"));
        security.clear_auth_failures("source:token");
        assert!(security.rate_limit_allows("source:token"));
    }

    #[test]
    fn authentication_failures_are_limited_by_source_even_when_token_ids_change() {
        let security = RemoteSecurity::for_tests();
        assert!(security.authentication_failure_rate_limit_allows("192.0.2.1", "token-one"));
        for index in 0..20 {
            security.record_authentication_failure("192.0.2.1", &format!("token-{index}"));
        }
        assert!(!security.authentication_failure_rate_limit_allows("192.0.2.1", "token-19"));
        assert!(!security.authentication_failure_rate_limit_allows("192.0.2.1", "rotated-token"));
        assert!(security.authentication_failure_rate_limit_allows("192.0.2.2", "token-two"));
    }

    #[test]
    fn wildcard_bind_requires_explicit_safeguard_but_public_ips_remain_rejected() {
        assert!(validate_bind("0.0.0.0".parse().unwrap(), false).is_err());
        assert!(validate_bind("0.0.0.0".parse().unwrap(), true).is_ok());
        assert!(validate_bind("203.0.113.10".parse().unwrap(), true).is_err());
    }

    #[test]
    fn pairing_failures_are_limited_by_source_even_when_codes_change() {
        let security = RemoteSecurity::for_tests();
        assert!(security.pairing_failure_rate_limit_allows("192.0.2.1", "first-invalid-code"));
        for index in 0..10 {
            security.record_pairing_failure("192.0.2.1", &format!("invalid-code-{index}"));
        }
        assert!(!security.pairing_failure_rate_limit_allows("192.0.2.1", "another-invalid-code"));
        let offer = security.create_pairing("https://backend.example.com", "AkironMux").unwrap();
        assert!(security.pairing_code_is_pending(&offer.code));
        assert!(security.pairing_failure_rate_limit_allows("192.0.2.2", "another-invalid-code"));
    }

    #[test]
    fn controller_can_recover_with_credential_but_other_connections_stay_read_only() {
        let leases = ControlLeases::default();
        let first = leases.connect("session", "first", "Desktop");
        let credential = first.recovery_credential.expect("controller receives a recovery credential");
        leases.disconnect("session", "first");

        let observer = leases.connect("session", "observer", "Phone");
        assert!(observer.recovery_credential.is_none());
        assert!(!leases.can_write("session", "observer"));

        let recovered = leases.connect("session", "reconnected", "Desktop");
        assert!(recovered.recovery_credential.is_none());
        let recovered = leases.recover_control("session", "reconnected", "Desktop", &credential);
        assert_eq!(recovered.state.controller_connection_id.as_deref(), Some("reconnected"));
        assert!(recovered.recovery_credential.is_some());
        assert!(leases.can_write("session", "reconnected"));
        assert!(!leases.can_write("session", "observer"));
    }

    #[test]
    fn expired_recovery_credential_cannot_restore_control() {
        let leases = ControlLeases::default();
        let first = leases.connect("session", "first", "Desktop");
        let credential = first.recovery_credential.unwrap();
        leases.disconnect("session", "first");
        leases.inner.lock().unwrap().recoveries.get_mut("session").unwrap().expires_at_ms = Some(now_ms() - 1);
        let recovered = leases.recover_control("session", "second", "Desktop", &credential);
        assert!(recovered.recovery_credential.is_none());
        assert!(!leases.can_write("session", "second"));
    }

    #[tokio::test]
    async fn pairing_requires_local_approval_before_issuing_token() {
        let security = RemoteSecurity::for_tests();
        let db = Db::open(Path::new(":memory:")).unwrap();
        let offer = security.create_pairing("https://backend.example.com", "AkironMux").unwrap();
        assert!(!offer.deep_link.contains("akmux_1_"));
        let claim = security.begin_pairing_claim(&offer.code, "Phone", "192.0.2.1").unwrap();
        assert_eq!(security.pending_pairings()[0].device_name.as_deref(), Some("Phone"));
        security.approve_pairing(&offer.id).unwrap();
        let device_name = security.finish_pairing(claim).await.unwrap();
        let (_, token) = security.create_device(&db, &device_name).unwrap();
        assert!(token.starts_with("akmux_1_"));
        assert!(security.begin_pairing_claim(&offer.code, "Replay", "192.0.2.1").is_err());
    }

    #[test]
    fn cancelled_pairing_code_is_immediately_invalid() {
        let security = RemoteSecurity::for_tests();
        let offer = security.create_pairing("https://backend.example.com", "AkironMux").unwrap();
        assert!(security.cancel_pairing(&offer.id));
        assert!(security.begin_pairing_claim(&offer.code, "Phone", "192.0.2.1").is_err());
        assert!(!security.cancel_pairing(&offer.id));
    }

    #[test]
    fn expired_pairing_and_concurrent_claims_are_rejected() {
        let security = RemoteSecurity::for_tests();
        let expired = security.create_pairing("https://backend.example.com", "AkironMux").unwrap();
        security.pairings.lock().unwrap().get_mut(&expired.id).unwrap().expires_at_ms = now_ms() - 1;
        assert!(security.begin_pairing_claim(&expired.code, "Phone", "192.0.2.1").is_err());

        let offer = security.create_pairing("https://backend.example.com", "AkironMux").unwrap();
        let barrier = Arc::new(std::sync::Barrier::new(3));
        let attempts = ["Phone", "Tablet"].map(|name| {
            let security = security.clone();
            let code = offer.code.clone();
            let barrier = Arc::clone(&barrier);
            std::thread::spawn(move || {
                barrier.wait();
                security.begin_pairing_claim(&code, name, "192.0.2.1").is_ok()
            })
        });
        barrier.wait();
        assert_eq!(attempts.into_iter().map(|attempt| attempt.join().unwrap()).filter(|claimed| *claimed).count(), 1);
    }
}
