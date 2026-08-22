//! Backend Profile security and persistence lifecycle.
//!
//! Tauri commands adapt WebView intents to this interface. Network discovery,
//! credential storage, identity confirmation, and profile persistence remain
//! ordered here so callers cannot accidentally bypass a security step.

use std::{
    collections::HashMap,
    sync::{Mutex, OnceLock},
    time::{Duration, Instant},
};

use serde::{Deserialize, Serialize};
use tauri::AppHandle;
use zeroize::Zeroizing;

use super::{
    credential_for_unchanged_profile, delete_credential, load_state, next_operation_id, parse_pairing_link, request_health, request_pairing_token, revoke_remote_device,
    save_state, store_credential, validate_profile, validate_profile_identity, validate_remote_capabilities, BackendHealth, BackendProfile, BackendProfileState, LOCAL_PROFILE_ID,
};

const IDENTITY_CHALLENGE_TTL: Duration = Duration::from_secs(120);

#[derive(Debug, Deserialize)]
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

#[derive(Debug, Serialize)]
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

fn pending_challenges() -> &'static Mutex<HashMap<String, PendingIdentityChallenge>> {
    static CHALLENGES: OnceLock<Mutex<HashMap<String, PendingIdentityChallenge>>> = OnceLock::new();
    CHALLENGES.get_or_init(|| Mutex::new(HashMap::new()))
}

fn profile_state_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

pub struct BackendProfileLifecycle {
    app: AppHandle,
}

impl BackendProfileLifecycle {
    pub fn new(app: AppHandle) -> Self {
        Self { app }
    }

    pub async fn apply(&self, intent: BackendProfileIntent) -> Result<BackendLifecycleOutcome, String> {
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

    async fn save(&self, mut profile: BackendProfile, pairing_link: String) -> Result<BackendLifecycleOutcome, String> {
        profile.name = profile.name.trim().to_string();
        validate_profile(&profile)?;
        let state = self.load_locked()?;
        validate_profile_identity(&state, &profile)?;
        let expected = state.profiles.iter().find(|current| current.id == profile.id).cloned();

        if profile.kind == "local" {
            profile.has_credential = false;
            profile.requires_auth = false;
            profile.instance_id = None;
            profile.capabilities.clear();
            return self.persist_profile(profile, expected);
        }

        if !pairing_link.trim().is_empty() {
            return self.pair(profile, expected, pairing_link, state).await;
        }

        let Some(credential) = credential_for_unchanged_profile(&state, &profile) else {
            return Ok(BackendLifecycleOutcome::AuthenticationRequired { state, profile_id: profile.id });
        };
        let health = request_health(&profile, Some(credential.as_str())).await?;
        validate_remote_capabilities(&health.capabilities)?;
        if identity_changed(&profile, &health) {
            return self.identity_challenge(state, PendingIdentityOperation::Save { profile, expected, health });
        }
        apply_health(&mut profile, health);
        self.persist_profile(profile, expected)
    }

    async fn pair(&self, profile: BackendProfile, expected: Option<BackendProfile>, pairing_link: String, state: BackendProfileState) -> Result<BackendLifecycleOutcome, String> {
        let parsed = parse_pairing_link(&pairing_link, &profile.address)?;
        let token = request_pairing_token(&profile, &parsed.code).await?;
        let health = request_health(&profile, Some(token.as_str())).await?;
        validate_remote_capabilities(&health.capabilities)?;
        if identity_changed(&profile, &health) {
            return self.identity_challenge(state, PendingIdentityOperation::Pair { profile, expected, health, token });
        }
        self.persist_paired_profile(profile, expected, health, token)
    }

    async fn select(&self, profile_id: String) -> Result<BackendLifecycleOutcome, String> {
        let state = self.load_locked()?;
        let profile = find_profile(&state, &profile_id)?;
        if profile.kind == "local" {
            return self.persist_selection(profile_id);
        }
        let Some(credential) = credential_for_unchanged_profile(&state, &profile) else {
            let state = self.persist_active_profile(profile_id.clone())?;
            return Ok(BackendLifecycleOutcome::AuthenticationRequired { state, profile_id });
        };
        let health = match request_health(&profile, Some(credential.as_str())).await {
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

    async fn refresh(&self, profile_id: String) -> Result<BackendLifecycleOutcome, String> {
        let state = self.load_locked()?;
        let profile = find_profile(&state, &profile_id)?;
        if profile.kind != "remote" {
            return Ok(BackendLifecycleOutcome::Applied { state });
        }
        let Some(credential) = credential_for_unchanged_profile(&state, &profile) else {
            return Ok(BackendLifecycleOutcome::AuthenticationRequired { state, profile_id });
        };
        let health = match request_health(&profile, Some(credential.as_str())).await {
            Ok(health) => health,
            Err(_) => return Ok(BackendLifecycleOutcome::Offline { state, profile_id }),
        };
        validate_remote_capabilities(&health.capabilities)?;
        if identity_changed(&profile, &health) {
            return self.identity_challenge(state, PendingIdentityOperation::Select { profile, health });
        }
        self.persist_profile_health(profile, health)
    }

    async fn delete(&self, profile_id: String) -> Result<BackendLifecycleOutcome, String> {
        if profile_id == LOCAL_PROFILE_ID {
            return Err("The built-in Local profile cannot be deleted".into());
        }
        let state = self.load_locked()?;
        let profile = find_profile(&state, &profile_id)?;
        let revocation_confirmed = profile.kind != "remote" || revoke_remote_device(&profile).await.is_ok();

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

    fn confirm_identity(&self, challenge_id: &str) -> Result<BackendLifecycleOutcome, String> {
        let challenge = take_challenge(challenge_id)?;
        match challenge.operation {
            PendingIdentityOperation::Save { mut profile, expected, health } => {
                apply_health(&mut profile, health);
                self.persist_profile(profile, expected)
            }
            PendingIdentityOperation::Pair { profile, expected, health, token } => self.persist_paired_profile(profile, expected, health, token),
            PendingIdentityOperation::Select { profile, health } => self.persist_selected_profile(profile, health),
        }
    }

    fn cancel_identity(&self, challenge_id: &str) -> Result<BackendLifecycleOutcome, String> {
        {
            let mut challenges = pending_challenges().lock().map_err(|_| "Identity confirmation state is unavailable".to_string())?;
            remove_expired_challenges(&mut challenges);
            challenges.remove(challenge_id);
        }
        Ok(BackendLifecycleOutcome::Applied { state: self.load_locked()? })
    }

    fn reorder(&self, profile_ids: Vec<String>) -> Result<BackendLifecycleOutcome, String> {
        let state = self.mutate_state(|state| {
            if profile_ids.len() != state.profiles.len() || !state.profiles.iter().all(|profile| profile_ids.contains(&profile.id)) {
                return Err("Backend profile order is invalid".into());
            }
            state
                .profiles
                .sort_by_key(|profile| profile_ids.iter().position(|id| id == &profile.id).unwrap_or(usize::MAX));
            Ok(())
        })?;
        Ok(BackendLifecycleOutcome::Applied { state })
    }

    fn identity_challenge(&self, state: BackendProfileState, operation: PendingIdentityOperation) -> Result<BackendLifecycleOutcome, String> {
        let (profile_id, observed_instance_id) = operation_identity(&operation);
        let challenge_id = next_operation_id();
        let mut challenges = pending_challenges().lock().map_err(|_| "Identity confirmation state is unavailable".to_string())?;
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
    ) -> Result<BackendLifecycleOutcome, String> {
        apply_health(&mut profile, health);
        store_credential(&profile.id, token.as_str())?;
        match self.persist_profile(profile.clone(), expected) {
            Ok(outcome) => Ok(outcome),
            Err(error) => {
                let _ = delete_credential(&profile.id);
                Err(error)
            }
        }
    }

    fn persist_selected_profile(&self, mut profile: BackendProfile, health: BackendHealth) -> Result<BackendLifecycleOutcome, String> {
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

    fn persist_profile_health(&self, mut profile: BackendProfile, health: BackendHealth) -> Result<BackendLifecycleOutcome, String> {
        let expected = Some(profile.clone());
        apply_health(&mut profile, health);
        self.persist_profile(profile, expected)
    }

    fn persist_profile(&self, profile: BackendProfile, expected: Option<BackendProfile>) -> Result<BackendLifecycleOutcome, String> {
        let profile_id = profile.id.clone();
        let state = self.mutate_state(|state| {
            ensure_profile_unchanged(state, &profile_id, expected.as_ref())?;
            validate_profile_identity(state, &profile)?;
            upsert_profile(state, profile);
            Ok(())
        })?;
        Ok(BackendLifecycleOutcome::Applied { state })
    }

    fn persist_selection(&self, profile_id: String) -> Result<BackendLifecycleOutcome, String> {
        Ok(BackendLifecycleOutcome::Applied {
            state: self.persist_active_profile(profile_id)?,
        })
    }

    fn persist_active_profile(&self, profile_id: String) -> Result<BackendProfileState, String> {
        self.mutate_state(|state| {
            if !state.profiles.iter().any(|profile| profile.id == profile_id) {
                return Err("Backend profile does not exist".into());
            }
            state.active_profile_id = profile_id;
            Ok(())
        })
    }

    fn delete_profile_locally(&self, expected: &BackendProfile) -> Result<BackendProfileState, String> {
        let _guard = profile_state_lock().lock().map_err(|_| "Backend Profile state is unavailable".to_string())?;
        let mut state = load_state(&self.app)?;
        ensure_profile_unchanged(&state, &expected.id, Some(expected))?;

        // Credentials are destroyed first. If metadata persistence then fails,
        // the surviving profile safely reloads as requiring authentication.
        delete_credential(&expected.id)?;
        state.profiles.retain(|profile| profile.id != expected.id);
        if state.active_profile_id == expected.id {
            state.active_profile_id = LOCAL_PROFILE_ID.into();
        }
        save_state(&self.app, &state)?;
        Ok(state)
    }

    fn load_locked(&self) -> Result<BackendProfileState, String> {
        let _guard = profile_state_lock().lock().map_err(|_| "Backend Profile state is unavailable".to_string())?;
        load_state(&self.app)
    }

    fn mutate_state(&self, mutation: impl FnOnce(&mut BackendProfileState) -> Result<(), String>) -> Result<BackendProfileState, String> {
        let _guard = profile_state_lock().lock().map_err(|_| "Backend Profile state is unavailable".to_string())?;
        let mut state = load_state(&self.app)?;
        mutation(&mut state)?;
        save_state(&self.app, &state)?;
        Ok(state)
    }
}

fn find_profile(state: &BackendProfileState, profile_id: &str) -> Result<BackendProfile, String> {
    state
        .profiles
        .iter()
        .find(|profile| profile.id == profile_id)
        .cloned()
        .ok_or_else(|| "Backend profile does not exist".into())
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

fn replace_profile(state: &mut BackendProfileState, profile: BackendProfile, expected: &BackendProfile) -> Result<(), String> {
    let Some(existing) = state.profiles.iter_mut().find(|item| item.id == profile.id) else {
        return Err("Backend profile changed while identity confirmation was pending".into());
    };
    if existing != expected {
        return Err("Backend profile changed while identity confirmation was pending".into());
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

fn take_challenge(challenge_id: &str) -> Result<PendingIdentityChallenge, String> {
    let mut challenges = pending_challenges().lock().map_err(|_| "Identity confirmation state is unavailable".to_string())?;
    remove_expired_challenges(&mut challenges);
    challenges.remove(challenge_id).ok_or_else(|| "Identity confirmation expired or was already used".into())
}

fn remove_expired_challenges(challenges: &mut HashMap<String, PendingIdentityChallenge>) {
    let now = Instant::now();
    challenges.retain(|_, challenge| challenge.expires_at > now);
}

fn ensure_profile_unchanged(state: &BackendProfileState, profile_id: &str, expected: Option<&BackendProfile>) -> Result<(), String> {
    let current = state.profiles.iter().find(|candidate| candidate.id == profile_id);
    match (expected, current) {
        (Some(expected), Some(current)) if current == expected => Ok(()),
        (None, None) => Ok(()),
        _ => Err("Backend profile changed while the lifecycle operation was pending".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_changes_only_after_an_expected_instance_was_pinned() {
        let mut profile = BackendProfileState::default().profiles.remove(0);
        let health = BackendHealth {
            instance_id: "observed".into(),
            api_protocol: "1.0".into(),
            capabilities: Vec::new(),
        };
        assert!(!identity_changed(&profile, &health));
        profile.instance_id = Some("expected".into());
        assert!(identity_changed(&profile, &health));
        profile.instance_id = Some("observed".into());
        assert!(!identity_changed(&profile, &health));
    }

    #[test]
    fn identity_challenges_are_single_use() {
        let challenge_id = next_operation_id();
        let profile = BackendProfileState::default().profiles.remove(0);
        let health = BackendHealth {
            instance_id: "observed".into(),
            api_protocol: "1.0".into(),
            capabilities: Vec::new(),
        };
        pending_challenges().lock().unwrap().insert(
            challenge_id.clone(),
            PendingIdentityChallenge {
                expires_at: Instant::now() + IDENTITY_CHALLENGE_TTL,
                operation: PendingIdentityOperation::Save { profile, expected: None, health },
            },
        );
        assert!(take_challenge(&challenge_id).is_ok());
        assert!(take_challenge(&challenge_id).is_err());
    }

    #[test]
    fn expired_identity_challenges_are_rejected() {
        let challenge_id = next_operation_id();
        let profile = BackendProfileState::default().profiles.remove(0);
        let health = BackendHealth {
            instance_id: "observed".into(),
            api_protocol: "1.0".into(),
            capabilities: Vec::new(),
        };
        pending_challenges().lock().unwrap().insert(
            challenge_id.clone(),
            PendingIdentityChallenge {
                expires_at: Instant::now() - Duration::from_millis(1),
                operation: PendingIdentityOperation::Save { profile, expected: None, health },
            },
        );
        assert!(take_challenge(&challenge_id).is_err());
    }

    #[test]
    fn lifecycle_outcomes_have_stable_typed_wire_names() {
        let value = serde_json::to_value(BackendLifecycleOutcome::IdentityConfirmationRequired {
            state: BackendProfileState::default(),
            challenge_id: "challenge".into(),
            profile_id: "remote".into(),
            observed_instance_id: "instance".into(),
        })
        .unwrap();
        assert_eq!(value["type"], "identityConfirmationRequired");
        assert_eq!(value["challengeId"], "challenge");
        assert_eq!(value["observedInstanceId"], "instance");
    }

    #[test]
    fn lifecycle_intents_accept_camel_case_fields() {
        let intent = serde_json::from_value::<BackendProfileIntent>(serde_json::json!({
            "type": "select",
            "profileId": "remote"
        }))
        .unwrap();
        assert!(matches!(intent, BackendProfileIntent::Select { profile_id } if profile_id == "remote"));
    }

    #[test]
    fn pending_operations_cannot_overwrite_a_changed_profile() {
        let mut state = BackendProfileState::default();
        let expected = state.profiles[0].clone();
        state.profiles[0].address = "http://127.0.0.1:17322".into();
        assert!(ensure_profile_unchanged(&state, &expected.id, Some(&expected)).is_err());
    }
}
