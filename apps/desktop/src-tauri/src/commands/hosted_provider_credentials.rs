//! Native OS-keyring management for an optional hosted compute provider.
//!
//! No get-secret IPC exists. Saving a credential is NOT export approval, an
//! Agents API compatibility check, or permission to create paid sessions.
use infimount_core::{CoreError, SecretStore, SecretStoreStatus};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tauri::State;

use crate::state::AppState;

const HOSTED_PROVIDER_ACCOUNT: &str = "openai-agents-api-primary-v1";
const MIN_KEY_LENGTH: usize = 16;
const MAX_KEY_LENGTH: usize = 1024;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SaveHostedProviderCredentialRequest {
    pub api_key: String,
}

/// No key, key prefix, key fingerprint, secret reference, or account value
/// may appear in this public response. Configured does not mean verified.
#[derive(Debug, Serialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct HostedProviderCredentialStatus {
    pub provider: &'static str,
    pub native_store_available: bool,
    pub configured: bool,
    pub verified: bool,
}

fn status(available: bool, configured: bool) -> HostedProviderCredentialStatus {
    HostedProviderCredentialStatus {
        provider: "openai_agents_api",
        native_store_available: available,
        configured,
        verified: false,
    }
}

fn validate_api_key(value: &str) -> Result<(), CoreError> {
    if value.len() < MIN_KEY_LENGTH
        || value.len() > MAX_KEY_LENGTH
        || !value.bytes().all(|byte| (0x21..=0x7e).contains(&byte))
    {
        return Err(CoreError::Config(
            "Hosted provider credential must be an opaque printable token without whitespace"
                .into(),
        ));
    }
    Ok(())
}

fn read_status(store: &dyn SecretStore) -> Result<HostedProviderCredentialStatus, CoreError> {
    if !matches!(store.status(), SecretStoreStatus::Available) {
        return Ok(status(false, false));
    }
    let bundle = store.get_json(HOSTED_PROVIDER_ACCOUNT).map_err(|_| {
        CoreError::Config("Native hosted provider credential status is unavailable".into())
    })?;
    let configured = match bundle {
        None => false,
        Some(value) => value
            .get("apiKey")
            .and_then(|v| v.as_str())
            .is_some_and(|key| validate_api_key(key).is_ok()),
    };
    Ok(status(true, configured))
}

fn save_key(
    store: &dyn SecretStore,
    api_key: String,
) -> Result<HostedProviderCredentialStatus, CoreError> {
    validate_api_key(&api_key)?;
    if !matches!(store.status(), SecretStoreStatus::Available) {
        return Err(CoreError::Config(
            "Native OS secret storage must be available to save a hosted provider key".into(),
        ));
    }
    store
        .put_json(HOSTED_PROVIDER_ACCOUNT, &json!({"apiKey": api_key}))
        .map_err(|_| {
            CoreError::Config("Native hosted provider credential could not be saved".into())
        })?;
    Ok(status(true, true))
}

fn delete_key(store: &dyn SecretStore) -> Result<HostedProviderCredentialStatus, CoreError> {
    if !matches!(store.status(), SecretStoreStatus::Available) {
        return Err(CoreError::Config(
            "Native OS secret storage must be available to remove a hosted provider key".into(),
        ));
    }
    store.delete(HOSTED_PROVIDER_ACCOUNT).map_err(|_| {
        CoreError::Config("Native hosted provider credential could not be removed".into())
    })?;
    Ok(status(true, false))
}

#[tauri::command]
pub async fn hosted_provider_credential_status(
    state: State<'_, AppState>,
) -> Result<HostedProviderCredentialStatus, CoreError> {
    let store = state.secret_store.clone();
    tauri::async_runtime::spawn_blocking(move || read_status(store.as_ref()))
        .await
        .map_err(|_| CoreError::Config("Native hosted provider status check failed".into()))?
}

#[tauri::command]
pub async fn save_hosted_provider_credential(
    state: State<'_, AppState>,
    request: SaveHostedProviderCredentialRequest,
) -> Result<HostedProviderCredentialStatus, CoreError> {
    // Serialize keyring changes against restore and startup recovery.
    // Never create/remove a secret while configuration state is being changed.
    let _lifecycle = state.lifecycle_mutation.lock().await;
    state.require_operational().map_err(|_| {
        CoreError::Config("Hosted provider credential changes are disabled during recovery".into())
    })?;
    let store = state.secret_store.clone();
    tauri::async_runtime::spawn_blocking(move || save_key(store.as_ref(), request.api_key))
        .await
        .map_err(|_| CoreError::Config("Native hosted provider save failed".into()))?
}

#[tauri::command]
pub async fn remove_hosted_provider_credential(
    state: State<'_, AppState>,
) -> Result<HostedProviderCredentialStatus, CoreError> {
    // Serialize keyring changes against restore and startup recovery.
    // Never create/remove a secret while configuration state is being changed.
    let _lifecycle = state.lifecycle_mutation.lock().await;
    state.require_operational().map_err(|_| {
        CoreError::Config("Hosted provider credential changes are disabled during recovery".into())
    })?;
    let store = state.secret_store.clone();
    tauri::async_runtime::spawn_blocking(move || delete_key(store.as_ref()))
        .await
        .map_err(|_| CoreError::Config("Native hosted provider removal failed".into()))?
}

#[cfg(test)]
mod tests {
    use super::*;
    use infimount_core::secrets::{MemorySecretStore, UnavailableSecretStore};

    #[test]
    fn credential_is_stored_only_in_native_secret_store_and_never_exposed_by_status() {
        let store = MemorySecretStore::new();
        assert_eq!(read_status(&store).unwrap(), status(true, false));
        let key = "sk-proj-test-opaque-key-123456789";
        let saved = save_key(&store, key.into()).unwrap();
        assert_eq!(saved, status(true, true));
        let response = serde_json::to_string(&saved).unwrap();
        assert!(!response.contains(key));
        assert!(!response.contains("secret"));
        assert!(!response.contains("apiKey"));
        assert_eq!(
            store.get_json(HOSTED_PROVIDER_ACCOUNT).unwrap().unwrap()["apiKey"],
            key
        );
        assert_eq!(read_status(&store).unwrap(), status(true, true));
        assert_eq!(delete_key(&store).unwrap(), status(true, false));
        assert_eq!(read_status(&store).unwrap(), status(true, false));
    }

    #[test]
    fn credential_input_denies_whitespace_control_and_oversized_values() {
        let store = MemorySecretStore::new();
        for value in [
            "",
            "short",
            "this key has spaces",
            "key\nwith-newline-123",
            "🍀",
        ] {
            assert!(save_key(&store, value.into()).is_err());
        }
        assert!(save_key(&store, "x".repeat(MAX_KEY_LENGTH + 1)).is_err());
        assert_eq!(read_status(&store).unwrap(), status(true, false));
    }

    #[test]
    fn unavailable_secret_store_never_falls_back_to_plaintext() {
        let store = UnavailableSecretStore::new("native keyring unavailable");
        assert_eq!(read_status(&store).unwrap(), status(false, false));
        assert!(save_key(&store, "sk-test-a-valid-opaque-secret".into()).is_err());
        assert!(delete_key(&store).is_err());
    }
}
