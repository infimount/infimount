use std::collections::{HashMap, HashSet};
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

pub fn discover_secret_field_names() -> HashSet<String> {
    let json_str = include_str!("../storage_schemas.json");
    let schemas: Vec<serde_json::Value> = match serde_json::from_str(json_str) {
        Ok(v) => v,
        Err(_) => return HashSet::new(),
    };

    let mut names = HashSet::new();
    for schema in &schemas {
        if let Some(fields) = schema.get("fields").and_then(|f| f.as_array()) {
            for field in fields {
                if field
                    .get("secret")
                    .and_then(|s| s.as_bool())
                    .unwrap_or(false)
                {
                    if let Some(name) = field.get("name").and_then(|n| n.as_str()) {
                        names.insert(name.to_string());
                    }
                }
            }
        }
    }
    names
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SecretPathSegment {
    Key(String),
    Index(usize),
}

impl SecretPathSegment {
    fn as_key(&self) -> String {
        match self {
            SecretPathSegment::Key(key) => key.clone(),
            SecretPathSegment::Index(index) => index.to_string(),
        }
    }

    fn as_index(&self) -> Option<usize> {
        match self {
            SecretPathSegment::Index(index) => Some(*index),
            SecretPathSegment::Key(key) => key.parse::<usize>().ok(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecretPath {
    pub segments: Vec<SecretPathSegment>,
}

fn escape_pointer_segment(segment: &str) -> String {
    segment.replace('~', "~0").replace('/', "~1")
}

fn unescape_pointer_segment(segment: &str) -> Result<String> {
    let mut out = String::with_capacity(segment.len());
    let mut chars = segment.chars();
    while let Some(ch) = chars.next() {
        if ch == '~' {
            match chars.next() {
                Some('0') => out.push('~'),
                Some('1') => out.push('/'),
                _ => {
                    return Err(CoreError::Config(
                        "secret path contains an invalid escape sequence".to_string(),
                    ))
                }
            }
        } else {
            out.push(ch);
        }
    }
    Ok(out)
}

fn parse_pointer_path(path: &str) -> Result<SecretPath> {
    let mut segments = Vec::new();
    for raw in path.split('/').skip(1) {
        let segment = unescape_pointer_segment(raw)?;
        if let Ok(index) = segment.parse::<usize>() {
            segments.push(SecretPathSegment::Index(index));
        } else {
            segments.push(SecretPathSegment::Key(segment));
        }
    }
    Ok(SecretPath { segments })
}

fn push_numeric_or_key(segments: &mut Vec<SecretPathSegment>, segment: String) {
    if let Ok(index) = segment.parse::<usize>() {
        segments.push(SecretPathSegment::Index(index));
    } else {
        segments.push(SecretPathSegment::Key(segment));
    }
}

fn parse_escaped_dot_path(path: &str) -> Result<SecretPath> {
    let mut segments = Vec::new();
    let mut current = String::new();
    let mut chars = path.chars();
    while let Some(ch) = chars.next() {
        match ch {
            '\\' => match chars.next() {
                Some('\\') => current.push('\\'),
                Some('.') => current.push('.'),
                _ => {
                    return Err(CoreError::Config(
                        "secret path contains an invalid escape sequence".to_string(),
                    ))
                }
            },
            '.' => push_numeric_or_key(&mut segments, std::mem::take(&mut current)),
            _ => current.push(ch),
        }
    }
    push_numeric_or_key(&mut segments, current);
    Ok(SecretPath { segments })
}

/// Parse either a canonical RFC 6901 JSON Pointer or a legacy escaped-dot path.
pub fn parse_secret_path(path: &str) -> Result<SecretPath> {
    if path.starts_with('/') {
        parse_pointer_path(path)
    } else {
        parse_escaped_dot_path(path)
    }
}

/// Serialize a secret path canonically as an RFC 6901 JSON Pointer.
pub fn canonical_secret_path(path: &SecretPath) -> String {
    let mut out = String::new();
    for segment in &path.segments {
        out.push('/');
        out.push_str(&escape_pointer_segment(&segment.as_key()));
    }
    out
}

/// Normalize a secret field reference from any supported representation
/// (RFC 6901 pointer or escaped-dot path) to canonical RFC 6901.
pub fn canonicalize_secret_field(field: &str) -> String {
    parse_secret_path(field)
        .map(|path| canonical_secret_path(&path))
        .unwrap_or_else(|_| field.to_string())
}

/// Canonicalize every key in a stored secret bundle in place so callers can
/// compare bundle keys against canonical secret-field references.
pub fn canonicalize_bundle_keys(bundle: &mut Value) {
    if let Some(map) = bundle.as_object_mut() {
        let entries: Vec<(String, Value)> = std::mem::take(map).into_iter().collect();
        for (key, value) in entries {
            let canonical = parse_secret_path(&key)
                .map(|path| canonical_secret_path(&path))
                .unwrap_or_else(|_| key);
            map.insert(canonical, value);
        }
    }
}

fn tokenize_secret_name(key: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    for ch in key.chars() {
        if ch == '_' || ch == '-' || ch == '.' || ch == ' ' {
            if !current.is_empty() {
                tokens.push(current.to_ascii_lowercase());
                current.clear();
            }
            continue;
        }
        if ch.is_uppercase() {
            if let Some(prev) = current.chars().last() {
                if prev.is_lowercase() || prev.is_ascii_digit() {
                    tokens.push(current.to_ascii_lowercase());
                    current.clear();
                }
            }
        }
        current.push(ch);
    }
    if !current.is_empty() {
        tokens.push(current.to_ascii_lowercase());
    }
    tokens
}

const SENSITIVE_TOKEN_PATTERNS: &[&[&str]] = &[
    &["api", "key"],
    &["api", "key", "id"],
    &["access", "key"],
    &["access", "key", "id"],
    &["access", "key", "path"],
    &["secret", "access", "key"],
    &["secret", "key"],
    &["auth", "code"],
    &["auth", "token"],
    &["oauth", "client", "secret"],
    &["oauth", "token"],
    &["access", "token"],
    &["refresh", "token"],
    &["session", "token"],
    &["client", "secret"],
    &["password"],
    &["passphrase"],
    &["secret"],
    &["token"],
    &["key"],
    &["key", "id"],
    &["key", "path"],
    &["credential"],
    &["credentials"],
    &["private", "key"],
    &["private", "key", "path"],
    &["service", "account", "json"],
    &["code", "verifier"],
    &["device", "code"],
    &["application", "key"],
    &["application", "key", "id"],
];

fn tokens_contain_pattern(tokens: &[String], pattern: &[&str]) -> bool {
    if pattern.is_empty() || pattern.len() > tokens.len() {
        return false;
    }
    if pattern.len() == 1 {
        return tokens.len() == 1 && tokens[0] == pattern[0];
    }
    tokens.windows(pattern.len()).any(|window| {
        window
            .iter()
            .map(|token| token.as_str())
            .eq(pattern.iter().copied())
    })
}

/// One shared secret-field classifier. Schema-declared names always win, then
/// normalized token patterns. A single-token pattern must match the entire key
/// (so `keyframe`, `monkey`, and `tokenBucketSize` stay public); multi-token
/// patterns match as contiguous token subsequences.
pub fn is_secret_field_name(key: &str, schema_secret_names: &HashSet<String>) -> bool {
    let tokens = tokenize_secret_name(key);
    if tokens.is_empty() {
        return false;
    }
    schema_secret_names
        .iter()
        .any(|schema_name| tokenize_secret_name(schema_name) == tokens)
        || SENSITIVE_TOKEN_PATTERNS
            .iter()
            .any(|pattern| tokens_contain_pattern(&tokens, pattern))
}

fn schema_field_tokens_match(key: &str, schema_secret_names: &HashSet<String>) -> bool {
    let tokens = tokenize_secret_name(key);
    if tokens.is_empty() {
        return false;
    }
    schema_secret_names
        .iter()
        .any(|schema_name| tokenize_secret_name(schema_name) == tokens)
}

pub fn extract_secret_fields(
    config: &Value,
    schema_secret_names: &HashSet<String>,
) -> Vec<(String, Value)> {
    fn visit(
        value: &Value,
        path: &mut Vec<SecretPathSegment>,
        schema_names: &HashSet<String>,
        output: &mut Vec<(String, Value)>,
    ) {
        match value {
            Value::Object(map) => {
                for (key, child) in map {
                    let schema_secret = schema_field_tokens_match(key, schema_names);
                    let fallback_secret = !child.is_object()
                        && !child.is_array()
                        && is_secret_field_name(key, schema_names)
                        && !schema_secret;
                    let secret = schema_secret || fallback_secret;
                    if secret {
                        if !child.is_null() && !is_masked_value(child) {
                            path.push(SecretPathSegment::Key(key.clone()));
                            output.push((
                                canonical_secret_path(&SecretPath {
                                    segments: path.clone(),
                                }),
                                child.clone(),
                            ));
                            path.pop();
                        }
                    } else {
                        path.push(SecretPathSegment::Key(key.clone()));
                        visit(child, path, schema_names, output);
                        path.pop();
                    }
                }
            }
            Value::Array(items) => {
                for (index, child) in items.iter().enumerate() {
                    path.push(SecretPathSegment::Index(index));
                    visit(child, path, schema_names, output);
                    path.pop();
                }
            }
            _ => {}
        }
    }
    let mut extracted = Vec::new();
    visit(config, &mut Vec::new(), schema_secret_names, &mut extracted);
    extracted
}

pub fn strip_secret_fields(config: &mut Value, schema_secret_names: &HashSet<String>) {
    fn visit(value: &mut Value, schema_names: &HashSet<String>) {
        match value {
            Value::Object(map) => {
                let keys = map.keys().cloned().collect::<Vec<_>>();
                for key in keys {
                    let schema_secret = schema_field_tokens_match(&key, schema_names);
                    let child_secret = map.get(&key).is_some_and(|child| {
                        !child.is_object()
                            && !child.is_array()
                            && is_secret_field_name(&key, schema_names)
                            && !schema_secret
                    });
                    if schema_secret || child_secret {
                        map.remove(&key);
                    } else if let Some(child) = map.get_mut(&key) {
                        visit(child, schema_names);
                    }
                }
            }
            Value::Array(items) => {
                for child in items.iter_mut() {
                    visit(child, schema_names);
                }
            }
            _ => {}
        }
    }
    visit(config, schema_secret_names);
}

/// Drop empty objects and arrays that remain after secret stripping so a
/// secret-only change never alters namespace identity. Used by fingerprinting,
/// not by stored configs, which must keep parent containers so secret paths
/// keep their object/array typing when merged back.
pub fn prune_empty_containers(value: &mut Value) {
    match value {
        Value::Object(map) => {
            let keys = map.keys().cloned().collect::<Vec<_>>();
            for key in keys {
                let emptied = map
                    .get_mut(&key)
                    .map(|child| {
                        prune_empty_containers(child);
                        matches!(child, Value::Object(m) if m.is_empty())
                            || matches!(child, Value::Array(a) if a.is_empty())
                    })
                    .unwrap_or(false);
                if emptied {
                    map.remove(&key);
                }
            }
        }
        Value::Array(items) => {
            for child in items.iter_mut() {
                prune_empty_containers(child);
            }
            items.retain(|child| {
                !(child.is_object() && child.as_object().unwrap().is_empty()
                    || child.is_array() && child.as_array().unwrap().is_empty())
            });
        }
        _ => {}
    }
}

pub fn merge_secret_config(public: &Value, secret_bundle: &Value) -> Value {
    fn insert_value(target: &mut Value, segments: &[SecretPathSegment], value: Value) {
        let Some((segment, rest)) = segments.split_first() else {
            *target = value;
            return;
        };
        // Preserve the type of an existing container. This distinguishes an
        // object key named "0" from array index 0; stripped public config keeps
        // its parent containers specifically so secret paths remain typed.
        if target.is_array() {
            let Some(index) = segment.as_index() else {
                return;
            };
            let array = target.as_array_mut().expect("array checked above");
            while array.len() <= index {
                array.push(Value::Null);
            }
            insert_value(&mut array[index], rest, value);
            return;
        }
        if !target.is_object() {
            *target = Value::Object(serde_json::Map::new());
        }
        let child = target
            .as_object_mut()
            .expect("object initialized above")
            .entry(segment.as_key())
            .or_insert(Value::Null);
        insert_value(child, rest, value);
    }

    let mut merged = public.clone();
    if let Some(secret_map) = secret_bundle.as_object() {
        for (path, value) in secret_map {
            if let Ok(parsed) = parse_secret_path(path) {
                insert_value(&mut merged, &parsed.segments, value.clone());
            }
        }
    }
    merged
}

/// Mask secret paths by parsing each one and walking the config type-aware.
/// Stale or unresolvable paths are skipped without panicking.
pub fn mask_secret_paths(config: &mut Value, paths: &[String]) {
    for path in paths {
        if let Ok(parsed) = parse_secret_path(path) {
            mask_path(config, &parsed.segments);
        }
    }
}

fn mask_path(value: &mut Value, segments: &[SecretPathSegment]) {
    let Some((segment, rest)) = segments.split_first() else {
        *value = Value::String("********".to_string());
        return;
    };
    match value {
        Value::Array(items) => {
            if let Some(index) = segment.as_index() {
                if let Some(child) = items.get_mut(index) {
                    mask_path(child, rest);
                }
            }
        }
        Value::Object(map) => {
            let key = segment.as_key();
            if let Some(child) = map.get_mut(&key) {
                mask_path(child, rest);
            } else {
                // The field was stripped as a secret; surface a mask in its place.
                map.insert(key, Value::String("********".to_string()));
            }
        }
        _ => {}
    }
}

pub fn contains_plaintext_secrets(config: &Value, schema_secret_names: &HashSet<String>) -> bool {
    !extract_secret_fields(config, schema_secret_names).is_empty()
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
        assert!(names.contains("accessKeyId"));
        assert!(names.contains("secretAccessKey"));
        assert!(names.contains("applicationKey"));
        assert!(names.contains("clientSecret"));
        assert!(names.contains("refreshToken"));
        assert!(names.contains("privateKeyPath"));
    }

    #[test]
    fn extract_secret_fields_from_config() {
        let schema_names: HashSet<String> = ["accessKeyId", "secretAccessKey"]
            .into_iter()
            .map(String::from)
            .collect();
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
        assert!(extracted_keys.contains(&"/accessKeyId"));
        assert!(extracted_keys.contains(&"/secretAccessKey"));
        assert!(extracted_keys.contains(&"/sessionToken"));
        assert!(!extracted_keys.contains(&"/bucket"));
        assert!(!extracted_keys.contains(&"/publicField"));
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
        let names: HashSet<String> = discover_secret_field_names().into_iter().collect();
        let extracted = extract_secret_fields(&config, &names);
        assert_eq!(extracted[0].0, "/advanced/credentials/refreshToken");
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
        let names: HashSet<String> = discover_secret_field_names().into_iter().collect();
        let extracted = extract_secret_fields(&config, &names);
        assert!(extracted
            .iter()
            .any(|(path, _)| path == "/profiles/0/credentials/accessToken"));
        assert!(extracted
            .iter()
            .any(|(path, _)| path == "/profiles/1/password"));
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
        let names: HashSet<String> = ["a.b", "0"].into_iter().map(String::from).collect();
        let extracted = extract_secret_fields(&config, &names);
        let paths = extracted
            .iter()
            .map(|(path, _)| path.as_str())
            .collect::<Vec<_>>();
        assert!(paths.contains(&"/a.b"));
        assert!(paths.contains(&"/nested/0"));
        assert!(paths.contains(&"/items/0/0"));

        let mut public = config.clone();
        strip_secret_fields(&mut public, &names);
        assert!(!contains_plaintext_secrets(&public, &names));
        let bundle = Value::Object(extracted.into_iter().collect());
        assert_eq!(merge_secret_config(&public, &bundle), config);

        let mut masked = config.clone();
        mask_secret_paths(&mut masked, &["/a.b".to_string(), "/nested/0".to_string()]);
        assert_eq!(
            masked.pointer("/a.b").and_then(Value::as_str),
            Some("********")
        );
        assert_eq!(masked["nested"]["0"], "********");
    }

    #[test]
    fn mask_nested_array_secret_does_not_panic() {
        let mut config = serde_json::json!({
            "profiles": [
                { "accessKeyId": "AKIA...", "label": "primary" },
                { "accessKeyId": "AKIA...", "label": "secondary" }
            ]
        });
        mask_secret_paths(&mut config, &["/profiles/0/accessKeyId".to_string()]);
        assert_eq!(config["profiles"][0]["accessKeyId"], "********");
        assert_eq!(config["profiles"][1]["accessKeyId"], "AKIA...");

        let mut stale = config.clone();
        mask_secret_paths(
            &mut stale,
            &["/missing/0/secret".to_string(), "/profiles/9/x".to_string()],
        );
        assert_eq!(stale["missing"], "********");
        assert_eq!(stale["profiles"], config["profiles"]);
    }

    #[test]
    fn extract_secret_fields_skips_masked() {
        let config = serde_json::json!({
            "accessKeyId": "********",
            "secretAccessKey": ""
        });
        let schema_names: HashSet<String> = ["accessKeyId", "secretAccessKey"]
            .into_iter()
            .map(String::from)
            .collect();
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
        let schema_names: HashSet<String> = ["accessKeyId", "secretAccessKey"]
            .into_iter()
            .map(String::from)
            .collect();
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
        let secret =
            serde_json::json!({"/accessKeyId": "AKIA123", "/secretAccessKey": "secret123"});
        let merged = merge_secret_config(&public, &secret);
        assert_eq!(merged["bucket"], "my-bucket");
        assert_eq!(merged["accessKeyId"], "AKIA123");
        assert_eq!(merged["secretAccessKey"], "secret123");
    }

    #[test]
    fn merge_secret_config_accepts_legacy_escaped_dot_keys() {
        let public = serde_json::json!({"advanced": {"credentials": {"endpoint": "x"}}});
        let secret = serde_json::json!({"advanced.credentials.refreshToken": "old"});
        let merged = merge_secret_config(&public, &secret);
        assert_eq!(merged["advanced"]["credentials"]["refreshToken"], "old");
    }

    #[test]
    fn is_secret_field_name_matches_cases() {
        let empty = HashSet::new();
        assert!(is_secret_field_name("accessKeyId", &empty));
        assert!(is_secret_field_name("secretAccessKey", &empty));
        assert!(is_secret_field_name("sessionToken", &empty));
        assert!(is_secret_field_name("refreshToken", &empty));
        assert!(is_secret_field_name("clientSecret", &empty));
        assert!(is_secret_field_name("privateKeyPath", &empty));
        assert!(is_secret_field_name("applicationKey", &empty));
        assert!(is_secret_field_name("application_key", &empty));
        assert!(is_secret_field_name("apiKey", &empty));
        assert!(is_secret_field_name("authToken", &empty));
        assert!(is_secret_field_name("oauthToken", &empty));
        assert!(is_secret_field_name("accessToken", &empty));
        assert!(is_secret_field_name("secretAccessKey", &empty));
        assert!(is_secret_field_name("applicationKey", &empty));
        assert!(is_secret_field_name("privateKeyPath", &empty));
        assert!(!is_secret_field_name("bucket", &empty));
        assert!(!is_secret_field_name("region", &empty));
        assert!(!is_secret_field_name("endpoint", &empty));
        assert!(is_secret_field_name("key", &empty));
        assert!(is_secret_field_name("keyId", &empty));
        assert!(is_secret_field_name("credential", &empty));
        assert!(is_secret_field_name("credentials", &empty));
        assert!(is_secret_field_name("token", &empty));
        assert!(!is_secret_field_name("monkey", &empty));
        assert!(!is_secret_field_name("keyboardLayout", &empty));
        assert!(!is_secret_field_name("tokenBucketSize", &empty));
        assert!(!is_secret_field_name("keyframe", &empty));
        assert!(!is_secret_field_name("secretariat", &empty));
        assert!(!is_secret_field_name("myTokenValue", &empty));

        let schema: HashSet<String> = ["accessKeyId"].into_iter().map(String::from).collect();
        assert!(is_secret_field_name("access_key_id", &schema));
    }
}
