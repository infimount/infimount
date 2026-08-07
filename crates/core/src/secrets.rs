use std::collections::HashMap;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::models::{CoreError, Result};

pub const KEYRING_SERVICE: &str = "com.infimount.credentials";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SecretStoreStatus {
    Available,
    Locked,
    Unavailable { reason: String },
}

pub trait SecretStore: std::fmt::Debug + Send + Sync {
    fn status(&self) -> SecretStoreStatus;
    fn put_json(&self, account: &str, value: &Value) -> Result<()>;
    fn get_json(&self, account: &str) -> Result<Option<Value>>;
    fn delete(&self, account: &str) -> Result<()>;
}

pub struct NativeSecretStore {
    access: Mutex<()>,
}

impl std::fmt::Debug for NativeSecretStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NativeSecretStore").finish()
    }
}

impl NativeSecretStore {
    pub fn new() -> Self {
        Self {
            access: Mutex::new(()),
        }
    }
}

impl Default for NativeSecretStore {
    fn default() -> Self {
        Self::new()
    }
}

impl SecretStore for NativeSecretStore {
    fn status(&self) -> SecretStoreStatus {
        let Ok(_guard) = self.access.lock() else {
            return SecretStoreStatus::Unavailable {
                reason: "native secret storage synchronization failed".to_string(),
            };
        };
        let entry = match keyring::Entry::new(KEYRING_SERVICE, "_status") {
            Ok(entry) => entry,
            Err(keyring::Error::NoStorageAccess(_)) => {
                return SecretStoreStatus::Unavailable {
                    reason: "keyring storage access denied".to_string(),
                }
            }
            Err(_) => {
                return SecretStoreStatus::Unavailable {
                    reason: "native secret storage is unavailable".to_string(),
                }
            }
        };
        match entry.get_password() {
            Ok(_) | Err(keyring::Error::NoEntry) => SecretStoreStatus::Available,
            Err(keyring::Error::NoStorageAccess(_)) => SecretStoreStatus::Locked,
            Err(_) => SecretStoreStatus::Unavailable {
                reason: "native secret storage is unavailable".to_string(),
            },
        }
    }

    fn put_json(&self, account: &str, value: &Value) -> Result<()> {
        let _guard = self.access.lock().map_err(|_| {
            CoreError::Config("native secret storage synchronization failed".to_string())
        })?;
        let payload = serde_json::to_string(value)
            .map_err(|_| CoreError::Config("failed to serialize secret bundle".to_string()))?;
        let entry = keyring::Entry::new(KEYRING_SERVICE, account)
            .map_err(|_| CoreError::Config("native secret storage is unavailable".to_string()))?;
        entry
            .set_password(&payload)
            .map_err(|_| CoreError::Config("failed to store secret bundle".to_string()))
    }

    fn get_json(&self, account: &str) -> Result<Option<Value>> {
        let _guard = self.access.lock().map_err(|_| {
            CoreError::Config("native secret storage synchronization failed".to_string())
        })?;
        let entry = keyring::Entry::new(KEYRING_SERVICE, account)
            .map_err(|_| CoreError::Config("native secret storage is unavailable".to_string()))?;
        match entry.get_password() {
            Ok(raw) => {
                let value: Value = serde_json::from_str(&raw).map_err(|_| {
                    CoreError::Config("stored secret bundle is invalid".to_string())
                })?;
                Ok(Some(value))
            }
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(_) => Err(CoreError::Config(
                "failed to read native secret storage".to_string(),
            )),
        }
    }

    fn delete(&self, account: &str) -> Result<()> {
        let _guard = self.access.lock().map_err(|_| {
            CoreError::Config("native secret storage synchronization failed".to_string())
        })?;
        let entry = keyring::Entry::new(KEYRING_SERVICE, account)
            .map_err(|_| CoreError::Config("native secret storage is unavailable".to_string()))?;
        match entry.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(_) => Err(CoreError::Config(
                "failed to delete native secret".to_string(),
            )),
        }
    }
}

pub struct MemorySecretStore {
    store: Mutex<HashMap<String, Value>>,
}

impl std::fmt::Debug for MemorySecretStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MemorySecretStore").finish()
    }
}

impl MemorySecretStore {
    pub fn new() -> Self {
        Self {
            store: Mutex::new(HashMap::new()),
        }
    }
}

impl Default for MemorySecretStore {
    fn default() -> Self {
        Self::new()
    }
}

impl SecretStore for MemorySecretStore {
    fn status(&self) -> SecretStoreStatus {
        SecretStoreStatus::Available
    }

    fn put_json(&self, account: &str, value: &Value) -> Result<()> {
        let mut store = self.store.lock().unwrap();
        store.insert(account.to_string(), value.clone());
        Ok(())
    }

    fn get_json(&self, account: &str) -> Result<Option<Value>> {
        let store = self.store.lock().unwrap();
        Ok(store.get(account).cloned())
    }

    fn delete(&self, account: &str) -> Result<()> {
        let mut store = self.store.lock().unwrap();
        store.remove(account);
        Ok(())
    }
}

pub struct UnavailableSecretStore {
    reason: String,
}

impl std::fmt::Debug for UnavailableSecretStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UnavailableSecretStore").finish()
    }
}

impl UnavailableSecretStore {
    pub fn new(reason: impl Into<String>) -> Self {
        Self {
            reason: reason.into(),
        }
    }
}

impl SecretStore for UnavailableSecretStore {
    fn status(&self) -> SecretStoreStatus {
        SecretStoreStatus::Unavailable {
            reason: self.reason.clone(),
        }
    }

    fn put_json(&self, _account: &str, _value: &Value) -> Result<()> {
        Err(CoreError::Config(format!(
            "secret store unavailable: {}",
            self.reason
        )))
    }

    fn get_json(&self, _account: &str) -> Result<Option<Value>> {
        Err(CoreError::Config(format!(
            "secret store unavailable: {}",
            self.reason
        )))
    }

    fn delete(&self, _account: &str) -> Result<()> {
        Err(CoreError::Config(format!(
            "secret store unavailable: {}",
            self.reason
        )))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StorageFieldSchema {
    pub name: String,
    #[serde(default)]
    pub secret: bool,
}

pub fn discover_secret_field_names() -> Vec<String> {
    let json_str = include_str!("../storage_schemas.json");
    let schemas: Vec<serde_json::Value> = match serde_json::from_str(json_str) {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };

    let mut names = Vec::new();
    for schema in &schemas {
        if let Some(fields) = schema.get("fields").and_then(|f| f.as_array()) {
            for field in fields {
                if field
                    .get("secret")
                    .and_then(|s| s.as_bool())
                    .unwrap_or(false)
                {
                    if let Some(name) = field.get("name").and_then(|n| n.as_str()) {
                        names.push(name.to_string());
                    }
                }
            }
        }
    }
    names.sort();
    names.dedup();
    names
}

fn escape_key_for_path(key: &str) -> String {
    key.replace('\\', "\\\\").replace('.', "\\.")
}

fn split_path(path: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut chars = path.chars();
    while let Some(ch) = chars.next() {
        match ch {
            '\\' => {
                if let Some(next) = chars.next() {
                    current.push(next);
                }
            }
            '.' => {
                parts.push(std::mem::take(&mut current));
            }
            _ => current.push(ch),
        }
    }
    parts.push(current);
    parts
}

pub fn extract_secret_fields(
    config: &Value,
    schema_secret_names: &[String],
) -> Vec<(String, Value)> {
    fn visit(
        value: &Value,
        path: &str,
        schema_names: &[String],
        output: &mut Vec<(String, Value)>,
    ) {
        match value {
            Value::Object(map) => {
                for (key, child) in map {
                    let child_path = if path.is_empty() {
                        escape_key_for_path(key)
                    } else {
                        format!("{}.{}", path, escape_key_for_path(key))
                    };
                    let schema_secret = schema_names.iter().any(|name| name == key);
                    let fallback_secret =
                        !child.is_object() && !child.is_array() && is_secret_key_name(key);
                    let secret = schema_secret || fallback_secret;
                    if secret {
                        if !child.is_null() && !is_masked_value(child) {
                            output.push((child_path, child.clone()));
                        }
                    } else {
                        visit(child, &child_path, schema_names, output);
                    }
                }
            }
            Value::Array(items) => {
                for (index, child) in items.iter().enumerate() {
                    let child_path = if path.is_empty() {
                        index.to_string()
                    } else {
                        format!("{path}.{index}")
                    };
                    visit(child, &child_path, schema_names, output);
                }
            }
            _ => {}
        }
    }
    let mut extracted = Vec::new();
    visit(config, "", schema_secret_names, &mut extracted);
    extracted
}

pub fn strip_secret_fields(config: &mut Value, schema_secret_names: &[String]) {
    fn visit(value: &mut Value, schema_names: &[String]) {
        match value {
            Value::Object(map) => {
                let keys = map.keys().cloned().collect::<Vec<_>>();
                for key in keys {
                    let secret = map.get(&key).is_some_and(|child| {
                        schema_names.iter().any(|name| name == &key)
                            || (!child.is_object() && !child.is_array() && is_secret_key_name(&key))
                    });
                    if secret {
                        map.remove(&key);
                    } else if let Some(child) = map.get_mut(&key) {
                        visit(child, schema_names);
                    }
                }
            }
            Value::Array(items) => {
                for child in items {
                    visit(child, schema_names);
                }
            }
            _ => {}
        }
    }
    visit(config, schema_secret_names);
}

pub fn merge_secret_config(public: &Value, secret_bundle: &Value) -> Value {
    fn insert_path(target: &mut Value, parts: &[&str], value: Value) {
        let Some((part, rest)) = parts.split_first() else {
            *target = value;
            return;
        };
        // Preserve the type of an existing container. This distinguishes an
        // object key named "0" from array index 0; stripped public config keeps
        // its parent containers specifically so secret paths remain typed.
        if target.is_array() {
            let Ok(index) = part.parse::<usize>() else {
                return;
            };
            let array = target.as_array_mut().expect("array checked above");
            while array.len() <= index {
                array.push(Value::Null);
            }
            insert_path(&mut array[index], rest, value);
            return;
        }
        if !target.is_object() {
            *target = Value::Object(serde_json::Map::new());
        }
        let child = target
            .as_object_mut()
            .expect("object initialized above")
            .entry((*part).to_string())
            .or_insert(Value::Null);
        insert_path(child, rest, value);
    }

    let mut merged = public.clone();
    if let Some(secret_map) = secret_bundle.as_object() {
        for (path, value) in secret_map {
            let segment_strings = split_path(path);
            let parts: Vec<_> = segment_strings.iter().map(|s| s.as_str()).collect();
            insert_path(&mut merged, &parts, value.clone());
        }
    }
    merged
}

pub fn contains_plaintext_secrets(config: &Value, schema_secret_names: &[String]) -> bool {
    !extract_secret_fields(config, schema_secret_names).is_empty()
}

fn is_secret_key_name(key: &str) -> bool {
    let lowered = key.to_ascii_lowercase();
    // Exact (case-insensitive) match only. The caller also checks user-provided
    // schema_secret_names, so broad substring matching is unnecessary and risks
    // false positives (e.g. a key named "mytokenvalue" would match "token").
    matches!(
        lowered.as_str(),
        "secret"
            | "password"
            | "token"
            | "credential"
            | "credentials"
            | "key"
            | "keyid"
            | "access_key"
            | "accesskey"
            | "accesskeyid"
            | "access_key_id"
            | "secret_key"
            | "secretkey"
            | "secretaccesskey"
            | "secret_access_key"
            | "client_secret"
            | "clientsecret"
            | "session_token"
            | "sessiontoken"
            | "applicationkey"
            | "applicationkeyid"
            | "application_key"
            | "application_key_id"
            | "privatekey"
            | "privatekeypath"
            | "private_key"
            | "private_key_path"
            | "keypath"
            | "key_path"
            | "serviceaccountjson"
            | "service_account_json"
            | "codeverifier"
            | "code_verifier"
            | "devicecode"
            | "device_code"
            | "authcode"
            | "auth_code"
            | "refreshtoken"
            | "refresh_token"
            | "accesstoken"
            | "access_token"
    )
}

fn is_masked_value(value: &Value) -> bool {
    matches!(value, Value::String(s) if s == "********" || s.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_secret_store_status_is_serialized_across_threads() {
        let store = std::sync::Arc::new(NativeSecretStore::new());
        let threads = (0..8)
            .map(|_| {
                let store = store.clone();
                std::thread::spawn(move || {
                    let _ = store.status();
                })
            })
            .collect::<Vec<_>>();
        for thread in threads {
            thread.join().expect("status thread");
        }
    }

    #[test]
    fn memory_secret_store_round_trip() {
        let store = MemorySecretStore::new();
        assert_eq!(store.status(), SecretStoreStatus::Available);

        let value = serde_json::json!({"key": "secret-value", "num": 42});
        store.put_json("test-account", &value).expect("put");
        let retrieved = store.get_json("test-account").expect("get");
        assert_eq!(retrieved, Some(value));

        let missing = store.get_json("no-such-account").expect("get missing");
        assert_eq!(missing, None);
    }

    #[test]
    fn memory_secret_store_delete() {
        let store = MemorySecretStore::new();
        let value = serde_json::json!({"secret": "data"});
        store.put_json("delete-me", &value).expect("put");
        store.delete("delete-me").expect("delete");
        let retrieved = store.get_json("delete-me").expect("get");
        assert_eq!(retrieved, None);
    }

    #[test]
    fn unavailable_secret_store_always_fails() {
        let store = UnavailableSecretStore::new("no keychain");
        assert_eq!(
            store.status(),
            SecretStoreStatus::Unavailable {
                reason: "no keychain".to_string()
            }
        );
        assert!(store.put_json("a", &serde_json::json!({})).is_err());
        assert!(store.get_json("a").is_err());
        assert!(store.delete("a").is_err());
    }

    #[test]
    fn discover_secret_fields_finds_all_schema_secrets() {
        let names = discover_secret_field_names();
        assert!(names.contains(&"accessKeyId".to_string()));
        assert!(names.contains(&"secretAccessKey".to_string()));
        assert!(names.contains(&"applicationKey".to_string()));
        assert!(names.contains(&"clientSecret".to_string()));
        assert!(names.contains(&"refreshToken".to_string()));
        assert!(names.contains(&"privateKeyPath".to_string()));
    }

    #[test]
    fn extract_secret_fields_from_config() {
        let schema_names = vec!["accessKeyId".to_string(), "secretAccessKey".to_string()];
        let config = serde_json::json!({
            "bucket": "my-bucket",
            "region": "us-east-1",
            "accessKeyId": "AKIA123",
            "secretAccessKey": "secret123",
            "sessionToken": "token123",
            "publicField": "hello"
        });

        let extracted = extract_secret_fields(&config, &schema_names);
        let extracted_keys: Vec<&str> = extracted.iter().map(|(k, _)| k.as_str()).collect();
        assert!(extracted_keys.contains(&"accessKeyId"));
        assert!(extracted_keys.contains(&"secretAccessKey"));
        assert!(extracted_keys.contains(&"sessionToken"));
        assert!(!extracted_keys.contains(&"bucket"));
        assert!(!extracted_keys.contains(&"publicField"));
    }

    #[test]
    fn nested_advanced_secrets_are_extracted_stripped_and_merged() {
        let config = serde_json::json!({
            "advanced": {
                "credentials": {
                    "refreshToken": "nested-refresh-token",
                    "endpoint": "https://example.invalid"
                }
            }
        });
        let names = discover_secret_field_names();
        let extracted = extract_secret_fields(&config, &names);
        assert_eq!(extracted[0].0, "advanced.credentials.refreshToken");
        let mut public = config.clone();
        strip_secret_fields(&mut public, &names);
        assert!(public
            .pointer("/advanced/credentials/refreshToken")
            .is_none());
        let bundle = Value::Object(extracted.into_iter().collect());
        assert_eq!(
            merge_secret_config(&public, &bundle)
                .pointer("/advanced/credentials/refreshToken")
                .and_then(Value::as_str),
            Some("nested-refresh-token")
        );
    }

    #[test]
    fn nested_arrays_are_extracted_stripped_and_restored() {
        let config = serde_json::json!({
            "profiles": [
                { "name": "one", "credentials": { "accessToken": "secret-one" } },
                { "name": "two", "password": "secret-two" }
            ]
        });
        let names = discover_secret_field_names();
        let extracted = extract_secret_fields(&config, &names);
        assert!(extracted
            .iter()
            .any(|(path, _)| path == "profiles.0.credentials.accessToken"));
        assert!(extracted
            .iter()
            .any(|(path, _)| path == "profiles.1.password"));
        let mut public = config.clone();
        strip_secret_fields(&mut public, &names);
        assert!(!contains_plaintext_secrets(&public, &names));
        let bundle = Value::Object(extracted.into_iter().collect());
        assert_eq!(merge_secret_config(&public, &bundle), config);
    }

    #[test]
    fn escaped_and_numeric_object_keys_round_trip_without_array_confusion() {
        let config = serde_json::json!({
            "a.b": "root-secret",
            "nested": { "0": "object-secret" },
            "items": [{ "0": "array-object-secret" }]
        });
        let names = vec!["a.b".to_string(), "0".to_string()];
        let extracted = extract_secret_fields(&config, &names);
        let paths = extracted
            .iter()
            .map(|(path, _)| path.as_str())
            .collect::<Vec<_>>();
        assert!(paths.contains(&r"a\.b"));
        assert!(paths.contains(&"nested.0"));
        assert!(paths.contains(&"items.0.0"));

        let mut public = config.clone();
        strip_secret_fields(&mut public, &names);
        let bundle = Value::Object(extracted.into_iter().collect());
        assert_eq!(merge_secret_config(&public, &bundle), config);
    }

    #[test]
    fn extract_secret_fields_skips_masked() {
        let config = serde_json::json!({
            "accessKeyId": "********",
            "secretAccessKey": ""
        });
        let schema_names = vec!["accessKeyId".to_string(), "secretAccessKey".to_string()];
        let extracted = extract_secret_fields(&config, &schema_names);
        assert!(extracted.is_empty());
    }

    #[test]
    fn strip_secret_fields_removes_schema_secrets() {
        let mut config = serde_json::json!({
            "bucket": "my-bucket",
            "accessKeyId": "AKIA123",
            "secretAccessKey": "secret123"
        });
        let schema_names = vec!["accessKeyId".to_string(), "secretAccessKey".to_string()];
        strip_secret_fields(&mut config, &schema_names);
        assert!(config.get("accessKeyId").is_none());
        assert!(config.get("secretAccessKey").is_none());
        assert_eq!(
            config.get("bucket").and_then(|v| v.as_str()),
            Some("my-bucket")
        );
    }

    #[test]
    fn merge_secret_config_combines_public_and_secret() {
        let public = serde_json::json!({"bucket": "my-bucket", "region": "us-east-1"});
        let secret = serde_json::json!({"accessKeyId": "AKIA123", "secretAccessKey": "secret123"});
        let merged = merge_secret_config(&public, &secret);
        assert_eq!(merged["bucket"], "my-bucket");
        assert_eq!(merged["accessKeyId"], "AKIA123");
        assert_eq!(merged["secretAccessKey"], "secret123");
    }

    #[test]
    fn is_secret_key_name_matches_cases() {
        assert!(is_secret_key_name("accessKeyId"));
        assert!(is_secret_key_name("secretAccessKey"));
        assert!(is_secret_key_name("sessionToken"));
        assert!(is_secret_key_name("refreshToken"));
        assert!(is_secret_key_name("clientSecret"));
        assert!(is_secret_key_name("privateKeyPath"));
        assert!(is_secret_key_name("applicationKey"));
        assert!(is_secret_key_name("application_key"));
        assert!(!is_secret_key_name("bucket"));
        assert!(!is_secret_key_name("region"));
        assert!(!is_secret_key_name("endpoint"));
        assert!(is_secret_key_name("key"));
        assert!(is_secret_key_name("keyId"));
        assert!(is_secret_key_name("credential"));
        assert!(is_secret_key_name("credentials"));
        assert!(is_secret_key_name("token"));
    }
}
