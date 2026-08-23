use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use serde::{Deserialize, Serialize};
use url::Url;
use zeroize::Zeroizing;

use crate::{
    BackendHealth, BackendKind, BackendProfile, BackendProfileState, ClientError, LOCAL_PROFILE_ID, Result,
    api::{BackendApi, validate_profile, validate_remote_capabilities},
    credentials::CredentialStore,
    storage::{ClientStateStore, operation_id},
};

const IDENTITY_CHALLENGE_TTL: Duration = Duration::from_secs(120);

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum BackendProfileIntent {
    Save { profile: BackendProfile, pairing_link: Option<String> },
    Select { profile_id: String },
    ConfirmIdentity { challenge_id: String },
    CancelIdentity { challenge_id: String },
    Reorder { profile_ids: Vec<String> },
    Delete { profile_id: String },
    Refresh { profile_id: String },
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum BackendLifecycleOutcome {
    Applied {
        state: BackendProfileState,
    },
    IdentityConfirmationRequired {
        state: BackendProfileState,
        challenge_id: String,
        profile_id: String,
        observed_instance_id: String,
    },
    AuthenticationRequired {
        state: BackendProfileState,
        profile_id: String,
    },
    Offline {
        state: BackendProfileState,
        profile_id: String,
    },
    AppliedWithWarning {
        state: BackendProfileState,
        warning: String,
    },
}

enum PendingIdentityOperation {
    Save {
        profile: BackendProfile,
        expected: Option<BackendProfile>,
        health: BackendHealth,
    },
    Pair {
        profile: BackendProfile,
        expected: Option<BackendProfile>,
        health: BackendHealth,
        token: Zeroizing<String>,
    },
    Select {
        profile: BackendProfile,
        health: BackendHealth,
    },
}

struct PendingIdentityChallenge {
    expires_at: Instant,
    operation: PendingIdentityOperation,
}

pub struct BackendProfileLifecycle {
    state: Arc<ClientStateStore>,
    credentials: Arc<dyn CredentialStore>,
    api: BackendApi,
    challenges: Mutex<HashMap<String, PendingIdentityChallenge>>,
    state_lock: Mutex<()>,
}

impl BackendProfileLifecycle {
    pub fn new(state: Arc<ClientStateStore>, credentials: Arc<dyn CredentialStore>, api: BackendApi) -> Self {
        Self {
            state,
            credentials,
            api,
            challenges: Mutex::new(HashMap::new()),
            state_lock: Mutex::new(()),
        }
    }

    pub fn state(&self) -> Result<BackendProfileState> {
        let _guard = self.state_lock.lock().map_err(|_| unavailable("Backend Profile state"))?;
        self.load_state()
    }

    pub async fn apply(&self, intent: BackendProfileIntent) -> Result<BackendLifecycleOutcome> {
        match intent {
            BackendProfileIntent::Save { profile, pairing_link } => self.save(profile, pairing_link.unwrap_or_default()).await,
            BackendProfileIntent::Select { profile_id } => self.select(profile_id).await,
            BackendProfileIntent::ConfirmIdentity { challenge_id } => self.confirm_identity(&challenge_id),
            BackendProfileIntent::CancelIdentity { challenge_id } => self.cancel_identity(&challenge_id),
            BackendProfileIntent::Reorder { profile_ids } => self.reorder(profile_ids),
            BackendProfileIntent::Delete { profile_id } => self.delete(profile_id).await,
            BackendProfileIntent::Refresh { profile_id } => self.refresh(profile_id).await,
        }
    }

    async fn save(&self, mut profile: BackendProfile, pairing_link: String) -> Result<BackendLifecycleOutcome> {
        profile.name = profile.name.trim().to_string();
        validate_profile(&profile)?;
        let state = self.state()?;
        validate_profile_identity(&state, &profile)?;
        let expected = state.profiles.iter().find(|current| current.id == profile.id).cloned();
        if profile.kind == BackendKind::Local {
            profile.has_credential = false;
            profile.requires_auth = false;
            profile.instance_id = None;
            profile.capabilities.clear();
            return self.persist_profile(profile, expected);
        }
        if !pairing_link.trim().is_empty() {
            return self.pair(profile, expected, &pairing_link, state).await;
        }
        let Some(credential) = self.credential_for_unchanged_profile(&state, &profile) else {
            return Ok(BackendLifecycleOutcome::AuthenticationRequired { state, profile_id: profile.id });
        };
        let health = self.api.health(&profile, Some(credential.as_str())).await?;
        validate_remote_capabilities(&health.capabilities)?;
        if identity_changed(&profile, &health) {
            return self.identity_challenge(state, PendingIdentityOperation::Save { profile, expected, health });
        }
        apply_health(&mut profile, health);
        self.persist_profile(profile, expected)
    }

    async fn pair(&self, profile: BackendProfile, expected: Option<BackendProfile>, pairing_link: &str, state: BackendProfileState) -> Result<BackendLifecycleOutcome> {
        let code = parse_pairing_link(pairing_link, &profile.address)?;
        let token = self.api.pair(&profile, &code).await?;
        let health = self.api.health(&profile, Some(token.as_str())).await?;
        validate_remote_capabilities(&health.capabilities)?;
        if identity_changed(&profile, &health) {
            return self.identity_challenge(state, PendingIdentityOperation::Pair { profile, expected, health, token });
        }
        self.persist_paired_profile(profile, expected, health, token)
    }

    async fn select(&self, profile_id: String) -> Result<BackendLifecycleOutcome> {
        let state = self.state()?;
        let profile = find_profile(&state, &profile_id)?;
        if profile.kind == BackendKind::Local {
            return self.persist_selection(profile_id);
        }
        let Some(credential) = self.credential_for_unchanged_profile(&state, &profile) else {
            let state = self.persist_active_profile(profile_id.clone())?;
            return Ok(BackendLifecycleOutcome::AuthenticationRequired { state, profile_id });
        };
        let health = match self.api.health(&profile, Some(credential.as_str())).await {
            Ok(health) => health,
            Err(_) => {
                let state = self.persist_active_profile(profile_id.clone())?;
                return Ok(BackendLifecycleOutcome::Offline { state, profile_id });
            }
        };
        validate_remote_capabilities(&health.capabilities)?;
        if identity_changed(&profile, &health) {
            return self.identity_challenge(state, PendingIdentityOperation::Select { profile, health });
        }
        self.persist_selected_profile(profile, health)
    }

    async fn refresh(&self, profile_id: String) -> Result<BackendLifecycleOutcome> {
        let state = self.state()?;
        let profile = find_profile(&state, &profile_id)?;
        if profile.kind != BackendKind::Remote {
            return Ok(BackendLifecycleOutcome::Applied { state });
        }
        let Some(credential) = self.credential_for_unchanged_profile(&state, &profile) else {
            return Ok(BackendLifecycleOutcome::AuthenticationRequired { state, profile_id });
        };
        let health = match self.api.health(&profile, Some(credential.as_str())).await {
            Ok(health) => health,
            Err(_) => return Ok(BackendLifecycleOutcome::Offline { state, profile_id }),
        };
        validate_remote_capabilities(&health.capabilities)?;
        if identity_changed(&profile, &health) {
            return self.identity_challenge(state, PendingIdentityOperation::Select { profile, health });
        }
        self.persist_profile_health(profile, health)
    }

    async fn delete(&self, profile_id: String) -> Result<BackendLifecycleOutcome> {
        if profile_id == LOCAL_PROFILE_ID {
            return Err(ClientError::Validation("The built-in Local profile cannot be deleted".into()));
        }
        let state = self.state()?;
        let profile = find_profile(&state, &profile_id)?;
        let revocation_confirmed = match profile.kind {
            BackendKind::Local => true,
            BackendKind::Remote => match self.credentials.load(&profile.id) {
                Ok(token) => self.api.revoke_current_device(&profile, token.as_str()).await.is_ok(),
                Err(_) => false,
            },
        };
        let state = self.delete_profile_locally(&profile)?;
        if revocation_confirmed {
            Ok(BackendLifecycleOutcome::Applied { state })
        } else {
            Ok(BackendLifecycleOutcome::AppliedWithWarning {
                state,
                warning: "Profile removed locally, but server-side device revocation could not be confirmed".into(),
            })
        }
    }

    fn confirm_identity(&self, challenge_id: &str) -> Result<BackendLifecycleOutcome> {
        let challenge = self.take_challenge(challenge_id)?;
        match challenge.operation {
            PendingIdentityOperation::Save { mut profile, expected, health } => {
                apply_health(&mut profile, health);
                self.persist_profile(profile, expected)
            }
            PendingIdentityOperation::Pair { profile, expected, health, token } => self.persist_paired_profile(profile, expected, health, token),
            PendingIdentityOperation::Select { profile, health } => self.persist_selected_profile(profile, health),
        }
    }

    fn cancel_identity(&self, challenge_id: &str) -> Result<BackendLifecycleOutcome> {
        let mut challenges = self.challenges.lock().map_err(|_| unavailable("Identity confirmation state"))?;
        remove_expired_challenges(&mut challenges);
        challenges.remove(challenge_id);
        drop(challenges);
        Ok(BackendLifecycleOutcome::Applied { state: self.state()? })
    }

    fn reorder(&self, profile_ids: Vec<String>) -> Result<BackendLifecycleOutcome> {
        let state = self.mutate_state(|state| {
            if profile_ids.len() != state.profiles.len() || !state.profiles.iter().all(|profile| profile_ids.contains(&profile.id)) {
                return Err(ClientError::Validation("Backend profile order is invalid".into()));
            }
            state
                .profiles
                .sort_by_key(|profile| profile_ids.iter().position(|id| id == &profile.id).unwrap_or(usize::MAX));
            Ok(())
        })?;
        Ok(BackendLifecycleOutcome::Applied { state })
    }

    fn identity_challenge(&self, state: BackendProfileState, operation: PendingIdentityOperation) -> Result<BackendLifecycleOutcome> {
        let (profile_id, observed_instance_id) = operation_identity(&operation);
        let challenge_id = operation_id();
        let mut challenges = self.challenges.lock().map_err(|_| unavailable("Identity confirmation state"))?;
        remove_expired_challenges(&mut challenges);
        challenges.insert(
            challenge_id.clone(),
            PendingIdentityChallenge {
                expires_at: Instant::now() + IDENTITY_CHALLENGE_TTL,
                operation,
            },
        );
        Ok(BackendLifecycleOutcome::IdentityConfirmationRequired {
            state,
            challenge_id,
            profile_id,
            observed_instance_id,
        })
    }

    fn persist_paired_profile(
        &self,
        mut profile: BackendProfile,
        expected: Option<BackendProfile>,
        health: BackendHealth,
        token: Zeroizing<String>,
    ) -> Result<BackendLifecycleOutcome> {
        apply_health(&mut profile, health);
        self.credentials.store(&profile.id, token.as_str())?;
        match self.persist_profile(profile.clone(), expected) {
            Ok(outcome) => Ok(outcome),
            Err(error) => {
                let _ = self.credentials.delete(&profile.id);
                Err(error)
            }
        }
    }

    fn persist_selected_profile(&self, mut profile: BackendProfile, health: BackendHealth) -> Result<BackendLifecycleOutcome> {
        let profile_id = profile.id.clone();
        let expected = profile.clone();
        apply_health(&mut profile, health);
        let state = self.mutate_state(|state| {
            replace_profile(state, profile, &expected)?;
            state.active_profile_id = profile_id;
            Ok(())
        })?;
        Ok(BackendLifecycleOutcome::Applied { state })
    }

    fn persist_profile_health(&self, mut profile: BackendProfile, health: BackendHealth) -> Result<BackendLifecycleOutcome> {
        let expected = Some(profile.clone());
        apply_health(&mut profile, health);
        self.persist_profile(profile, expected)
    }

    fn persist_profile(&self, profile: BackendProfile, expected: Option<BackendProfile>) -> Result<BackendLifecycleOutcome> {
        let profile_id = profile.id.clone();
        let state = self.mutate_state(|state| {
            ensure_profile_unchanged(state, &profile_id, expected.as_ref())?;
            validate_profile_identity(state, &profile)?;
            upsert_profile(state, profile);
            Ok(())
        })?;
        Ok(BackendLifecycleOutcome::Applied { state })
    }

    fn persist_selection(&self, profile_id: String) -> Result<BackendLifecycleOutcome> {
        Ok(BackendLifecycleOutcome::Applied {
            state: self.persist_active_profile(profile_id)?,
        })
    }

    fn persist_active_profile(&self, profile_id: String) -> Result<BackendProfileState> {
        self.mutate_state(|state| {
            if !state.profiles.iter().any(|profile| profile.id == profile_id) {
                return Err(ClientError::Validation("Backend profile does not exist".into()));
            }
            state.active_profile_id = profile_id;
            Ok(())
        })
    }

    fn delete_profile_locally(&self, expected: &BackendProfile) -> Result<BackendProfileState> {
        let _guard = self.state_lock.lock().map_err(|_| unavailable("Backend Profile state"))?;
        let mut state = self.load_state()?;
        ensure_profile_unchanged(&state, &expected.id, Some(expected))?;
        self.credentials.delete(&expected.id)?;
        state.profiles.retain(|profile| profile.id != expected.id);
        if state.active_profile_id == expected.id {
            state.active_profile_id = LOCAL_PROFILE_ID.into();
        }
        self.state.save_backend_profiles(&state)?;
        Ok(state)
    }

    fn mutate_state(&self, mutation: impl FnOnce(&mut BackendProfileState) -> Result<()>) -> Result<BackendProfileState> {
        let _guard = self.state_lock.lock().map_err(|_| unavailable("Backend Profile state"))?;
        let mut state = self.load_state()?;
        mutation(&mut state)?;
        self.state.save_backend_profiles(&state)?;
        Ok(state)
    }

    fn load_state(&self) -> Result<BackendProfileState> {
        let mut state = self.state.load_backend_profiles()?;
        if !state.profiles.iter().any(|profile| profile.id == LOCAL_PROFILE_ID) {
            state.profiles.insert(0, BackendProfile::local());
        }
        for profile in &mut state.profiles {
            validate_profile(profile)?;
            if profile.kind == BackendKind::Remote {
                let has_credential = self.credentials.load(&profile.id).is_ok();
                profile.has_credential = has_credential;
                profile.requires_auth = !has_credential;
            }
        }
        for index in 0..state.profiles.len() {
            if state.profiles[..index]
                .iter()
                .any(|existing| existing.name.trim().eq_ignore_ascii_case(state.profiles[index].name.trim()))
            {
                return Err(ClientError::InvalidState(serde_json::Error::io(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "Backend profile names are not unique",
                ))));
            }
        }
        if !state.profiles.iter().any(|profile| profile.id == state.active_profile_id) {
            state.active_profile_id = LOCAL_PROFILE_ID.into();
        }
        Ok(state)
    }

    fn credential_for_unchanged_profile(&self, state: &BackendProfileState, candidate: &BackendProfile) -> Option<Zeroizing<String>> {
        state
            .profiles
            .iter()
            .find(|profile| {
                profile.id == candidate.id && profile.kind == BackendKind::Remote && profile.address == candidate.address && profile.instance_id == candidate.instance_id
            })
            .and_then(|_| self.credentials.load(&candidate.id).ok())
    }

    fn take_challenge(&self, challenge_id: &str) -> Result<PendingIdentityChallenge> {
        let mut challenges = self.challenges.lock().map_err(|_| unavailable("Identity confirmation state"))?;
        remove_expired_challenges(&mut challenges);
        challenges
            .remove(challenge_id)
            .ok_or_else(|| ClientError::Validation("Identity confirmation expired or was already used".into()))
    }
}

fn parse_pairing_link(value: &str, expected_address: &str) -> Result<String> {
    let link = Url::parse(value.trim()).map_err(|_| ClientError::Validation("Pairing link is invalid".into()))?;
    if link.scheme() != "akmux" || link.host_str() != Some("pair") || !link.path().is_empty() {
        return Err(ClientError::Validation("Pairing link is invalid".into()));
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
    let address = Url::parse(
        address
            .as_deref()
            .ok_or_else(|| ClientError::Validation("Pairing link is missing its backend address".into()))?,
    )
    .map_err(|_| ClientError::Validation("Pairing link contains an invalid backend address".into()))?;
    let expected = Url::parse(expected_address).map_err(|_| ClientError::Validation("Backend address is invalid".into()))?;
    if address != expected {
        return Err(ClientError::Validation("Pairing link belongs to a different backend".into()));
    }
    code.filter(|value| !value.is_empty() && value.len() <= 256)
        .ok_or_else(|| ClientError::Validation("Pairing link is missing its code".into()))
}

fn find_profile(state: &BackendProfileState, profile_id: &str) -> Result<BackendProfile> {
    state
        .profiles
        .iter()
        .find(|profile| profile.id == profile_id)
        .cloned()
        .ok_or_else(|| ClientError::Validation("Backend profile does not exist".into()))
}

fn identity_changed(profile: &BackendProfile, health: &BackendHealth) -> bool {
    profile.instance_id.as_deref().is_some_and(|expected| expected != health.instance_id)
}

fn apply_health(profile: &mut BackendProfile, health: BackendHealth) {
    profile.has_credential = true;
    profile.requires_auth = false;
    profile.instance_id = Some(health.instance_id);
    profile.capabilities = health.capabilities;
}

fn upsert_profile(state: &mut BackendProfileState, profile: BackendProfile) {
    if let Some(existing) = state.profiles.iter_mut().find(|item| item.id == profile.id) {
        *existing = profile;
    } else {
        state.profiles.push(profile);
    }
}

fn replace_profile(state: &mut BackendProfileState, profile: BackendProfile, expected: &BackendProfile) -> Result<()> {
    let existing = state
        .profiles
        .iter_mut()
        .find(|item| item.id == profile.id)
        .ok_or_else(|| ClientError::Validation("Backend profile changed while identity confirmation was pending".into()))?;
    if existing != expected {
        return Err(ClientError::Validation("Backend profile changed while identity confirmation was pending".into()));
    }
    *existing = profile;
    Ok(())
}

fn operation_identity(operation: &PendingIdentityOperation) -> (String, String) {
    match operation {
        PendingIdentityOperation::Save { profile, health, .. } | PendingIdentityOperation::Pair { profile, health, .. } | PendingIdentityOperation::Select { profile, health } => {
            (profile.id.clone(), health.instance_id.clone())
        }
    }
}

fn remove_expired_challenges(challenges: &mut HashMap<String, PendingIdentityChallenge>) {
    let now = Instant::now();
    challenges.retain(|_, challenge| challenge.expires_at > now);
}

fn ensure_profile_unchanged(state: &BackendProfileState, profile_id: &str, expected: Option<&BackendProfile>) -> Result<()> {
    let current = state.profiles.iter().find(|candidate| candidate.id == profile_id);
    match (expected, current) {
        (Some(expected), Some(current)) if current == expected => Ok(()),
        (None, None) => Ok(()),
        _ => Err(ClientError::Validation("Backend profile changed while the lifecycle operation was pending".into())),
    }
}

fn validate_profile_identity(state: &BackendProfileState, profile: &BackendProfile) -> Result<()> {
    if state
        .profiles
        .iter()
        .any(|existing| existing.id != profile.id && existing.name.trim().eq_ignore_ascii_case(profile.name.trim()))
    {
        Err(ClientError::Validation("Backend profile names must be unique".into()))
    } else {
        Ok(())
    }
}

fn unavailable(label: &str) -> ClientError {
    ClientError::Platform(format!("{label} is unavailable"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{credentials::MemoryCredentialStore, storage::ClientPaths};

    fn lifecycle(directory: &std::path::Path) -> BackendProfileLifecycle {
        BackendProfileLifecycle::new(
            Arc::new(ClientStateStore::new(ClientPaths::new(directory))),
            Arc::new(MemoryCredentialStore::default()),
            BackendApi::new().unwrap(),
        )
    }

    #[test]
    fn local_profile_is_present_and_cannot_be_deleted_or_renamed() {
        let directory = tempfile::tempdir().unwrap();
        let lifecycle = lifecycle(directory.path());
        assert_eq!(lifecycle.state().unwrap(), BackendProfileState::default());
        let mut local = BackendProfile::local();
        local.name = "Changed".into();
        assert!(validate_profile(&local).is_err());
    }

    #[test]
    fn pairing_links_are_bound_to_the_expected_origin() {
        assert_eq!(
            parse_pairing_link("akmux://pair?url=https%3A%2F%2Fexample.com&code=abc", "https://example.com").unwrap(),
            "abc"
        );
        assert!(parse_pairing_link("akmux://pair?url=https%3A%2F%2Fother.example&code=abc", "https://example.com").is_err());
    }

    #[test]
    fn identity_changes_only_after_an_instance_is_pinned() {
        let mut profile = BackendProfile::local();
        let health = BackendHealth {
            instance_id: "observed".into(),
            api_protocol: "1.0".into(),
            capabilities: Vec::new(),
        };
        assert!(!identity_changed(&profile, &health));
        profile.instance_id = Some("expected".into());
        assert!(identity_changed(&profile, &health));
    }
}
