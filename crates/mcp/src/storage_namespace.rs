use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::errors::{err, err_with_details, McpErrorCode, McpResult};
use crate::registry::StorageRecord;
use infimount_core::registry::{
    normalize_endpoint_authority, resolve_namespace_fields, ResolvedNamespaceFields,
};
use infimount_core::{secrets, SourceKind};

/// Version of the namespace identity encoding. Persisted workspace fingerprints
/// are bound to this version; bumping it invalidates previously created workspaces.
pub const STORAGE_NAMESPACE_SCHEMA_VERSION: u32 = 1;

/// Canonical identity of the underlying storage namespace a workspace grant
/// refers to. Contains no secret values and no record identity (id/name/revision).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct StorageNamespaceDescriptor {
    pub version: u32,
    pub backend: String,
    pub authority: String,
    pub container: String,
    pub root: String,
    pub canonical_public_config_sha256: String,
}

/// A canonical namespace-absolute address for a single backend path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StorageNamespaceAddress {
    pub namespace_key: String,
    pub absolute_path: String,
}

/// Relationship between a source and a destination transfer endpoint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransferNamespaceRelation {
    /// True only when both sides resolve through the same operator scope
    /// (same storage record). Used only for server-side copy/rename optimization.
    pub same_operator_scope: bool,
    /// True when both sides land on the same underlying namespace, so the
    /// canonical namespace-absolute paths can be safely compared.
    pub same_underlying_namespace: bool,
    pub source_absolute_path: Option<String>,
    pub destination_absolute_path: Option<String>,
}

pub fn storage_namespace_descriptor(
    storage: &StorageRecord,
) -> McpResult<StorageNamespaceDescriptor> {
    let (kind, fields) = resolve_fields(storage)?;
    let public_config = public_config_for_fingerprint(storage)?;
    let canonical_config = canonical_json(&public_config);
    let config_bytes = serde_json::to_vec(&canonical_config).map_err(|_| {
        err(
            McpErrorCode::ERR_INTERNAL,
            "failed to serialize storage namespace identity",
        )
    })?;
    Ok(StorageNamespaceDescriptor {
        version: STORAGE_NAMESPACE_SCHEMA_VERSION,
        backend: kind.to_string(),
        authority: fields.authority,
        container: fields.container,
        root: fields.root,
        canonical_public_config_sha256: sha256_hex(&config_bytes),
    })
}

pub fn storage_namespace_fingerprint(storage: &StorageRecord) -> McpResult<String> {
    let descriptor = storage_namespace_descriptor(storage)?;
    let canonical = canonical_json(&serde_json::to_value(&descriptor).map_err(|_| {
        err(
            McpErrorCode::ERR_INTERNAL,
            "failed to serialize storage namespace descriptor",
        )
    })?);
    let bytes = serde_json::to_vec(&canonical).map_err(|_| {
        err(
            McpErrorCode::ERR_INTERNAL,
            "failed to serialize storage namespace identity",
        )
    })?;
    Ok(sha256_hex(&bytes))
}

/// Resolve a single backend path to a canonical namespace-absolute address.
/// Returns `None` for backends that cannot be pinned to a namespace from the
/// available non-secret identity (currently impossible; all backends resolve).
pub fn storage_namespace_address(
    storage: &StorageRecord,
    backend_path: &str,
) -> McpResult<Option<StorageNamespaceAddress>> {
    let (kind, mut fields) = resolve_fields(storage)?;
    fields.authority = infimount_core::registry::canonicalize_provider_default_authority(
        &kind,
        &storage.config,
        &fields.authority,
    );
    let key = namespace_key(&kind, &fields, storage);
    let Some(key) = key else {
        return Ok(None);
    };
    let absolute_path = build_absolute_path(&kind, &fields.root, backend_path)?;
    Ok(Some(StorageNamespaceAddress {
        namespace_key: key,
        absolute_path,
    }))
}

pub fn transfer_namespace_relation(
    source: &StorageRecord,
    source_backend_path: &str,
    destination: &StorageRecord,
    destination_backend_path: &str,
) -> McpResult<TransferNamespaceRelation> {
    let same_operator_scope = source.id == destination.id;
    let source_address = storage_namespace_address(source, source_backend_path)?;
    let destination_address = storage_namespace_address(destination, destination_backend_path)?;
    let same_underlying_namespace = match (&source_address, &destination_address) {
        (Some(source_addr), Some(destination_addr)) => {
            let both_local =
                is_local_backend(&source.backend) && is_local_backend(&destination.backend);
            both_local || source_addr.namespace_key == destination_addr.namespace_key
        }
        _ => false,
    };
    Ok(TransferNamespaceRelation {
        same_operator_scope,
        same_underlying_namespace,
        source_absolute_path: source_address.map(|address| address.absolute_path),
        destination_absolute_path: destination_address.map(|address| address.absolute_path),
    })
}

/// Returns true when a copy/move must be rejected before destination creation:
/// source absolute path equals destination, or destination is a descendant of
/// source within the same underlying namespace.
pub fn transfer_has_namespace_conflict(relation: &TransferNamespaceRelation) -> bool {
    if !relation.same_underlying_namespace {
        return false;
    }
    let (Some(source), Some(destination)) = (
        relation.source_absolute_path.as_deref(),
        relation.destination_absolute_path.as_deref(),
    ) else {
        return false;
    };
    paths_equal(source, destination) || path_is_descendant(source, destination)
}

fn resolve_fields(storage: &StorageRecord) -> McpResult<(SourceKind, ResolvedNamespaceFields)> {
    let kind = backend_kind(&storage.backend)?;
    let mut fields = resolve_namespace_fields(&kind, "", &storage.config);
    if matches!(kind, SourceKind::Local) {
        fields.root = canonical_local_root(&fields.root)?;
    } else {
        fields.authority = normalize_endpoint_authority(&fields.authority);
        fields.container = fields.container.trim().trim_matches('/').to_string();
        fields.root = normalize_root_prefix(&fields.root);
    }
    Ok((kind, fields))
}

fn backend_kind(backend: &str) -> McpResult<SourceKind> {
    backend.trim().parse::<SourceKind>().map_err(|_| {
        err_with_details(
            McpErrorCode::ERR_BACKEND_UNSUPPORTED,
            format!("unsupported backend '{backend}'"),
            serde_json::json!({ "backend": backend }),
        )
    })
}

fn is_local_backend(backend: &str) -> bool {
    matches!(backend.trim().to_ascii_lowercase().as_str(), "local" | "fs")
}

fn namespace_key(
    kind: &SourceKind,
    fields: &ResolvedNamespaceFields,
    storage: &StorageRecord,
) -> Option<String> {
    let backend = kind.to_string();
    match *kind {
        SourceKind::Local => Some(format!("{backend}://")),
        SourceKind::S3
        | SourceKind::WebDav
        | SourceKind::AzureBlob
        | SourceKind::Gcs
        | SourceKind::B2
        | SourceKind::Oss
        | SourceKind::Cos
        | SourceKind::Obs
        | SourceKind::Sftp
        | SourceKind::Ftp => Some(format!(
            "{backend}://{}/{}",
            fields.authority, fields.container
        )),
        SourceKind::Gdrive | SourceKind::Onedrive => {
            // No stable public account identity is available from the schema.
            // A drive account is pinned by the provider and the non-empty
            // secret reference; different secret references are unknown, never
            // automatically equal. The namespace key itself is never persisted.
            let account = storage
                .secret_ref
                .as_deref()
                .filter(|value| !value.is_empty())?;
            Some(format!("{backend}://{account}"))
        }
    }
}

/// Canonical root + operation path. Local paths use filesystem semantics;
/// object/webdav/remote keys use normalized prefix semantics. Returns an error
/// when the operation path would escape the namespace root.
fn build_absolute_path(kind: &SourceKind, root: &str, backend_path: &str) -> McpResult<String> {
    let normalized = normalize_operation_path(backend_path)?;
    if matches!(kind, SourceKind::Local) {
        let joined = join_within_root(root, &normalized).ok_or_else(|| {
            err_with_details(
                McpErrorCode::ERR_INVALID_PATH,
                "operation path escapes the storage namespace",
                serde_json::json!({}),
            )
        })?;
        return Ok(joined);
    }
    let mut segments: Vec<String> = if root.is_empty() {
        Vec::new()
    } else {
        root.trim_matches('/')
            .split('/')
            .map(String::from)
            .collect()
    };
    for segment in normalized.split('/') {
        match segment {
            "" | "." => {}
            ".." => {
                if segments.pop().is_none() {
                    return Err(err_with_details(
                        McpErrorCode::ERR_INVALID_PATH,
                        "operation path escapes the storage namespace",
                        serde_json::json!({}),
                    ));
                }
            }
            other => segments.push(other.to_string()),
        }
    }
    if segments.is_empty() {
        return Ok(String::new());
    }
    Ok(format!("/{}", segments.join("/")))
}

fn normalize_operation_path(backend_path: &str) -> McpResult<String> {
    if backend_path.contains('\\') {
        return Err(err(
            McpErrorCode::ERR_INVALID_PATH,
            "operation path must use forward slashes",
        ));
    }
    let mut segments: Vec<&str> = Vec::new();
    for segment in backend_path.split('/') {
        match segment {
            "" | "." => {}
            ".." => {
                return Err(err_with_details(
                    McpErrorCode::ERR_INVALID_PATH,
                    "operation path must not contain '..' segments",
                    serde_json::json!({}),
                ));
            }
            other => segments.push(other),
        }
    }
    Ok(segments.join("/"))
}

/// Join a relative path below a filesystem root, rejecting any `..` that would
/// escape the root. Local paths are compared on canonical roots so an alias
/// through a nested root is still caught.
fn join_within_root(root: &str, relative: &str) -> Option<String> {
    let root_path = Path::new(root);
    let mut depth = root_path.components().count();
    let mut base = PathBuf::from(root_path);
    for segment in relative.split('/') {
        match segment {
            "" | "." => {}
            ".." => {
                if depth == 0 || !base.pop() {
                    return None;
                }
                depth -= 1;
            }
            other => {
                base.push(other);
                depth += 1;
            }
        }
    }
    Some(base.to_string_lossy().to_string())
}

fn path_is_descendant(parent: &str, child: &str) -> bool {
    let parent = normalize_compare(parent);
    let child = normalize_compare(child);
    if child == parent {
        return false;
    }
    if parent.is_empty() {
        return true;
    }
    child.starts_with(&parent) && child.as_bytes().get(parent.len()) == Some(&b'/')
}

fn paths_equal(left: &str, right: &str) -> bool {
    normalize_compare(left) == normalize_compare(right)
}

fn normalize_compare(value: &str) -> String {
    #[cfg(windows)]
    {
        value
            .replace('\\', "/")
            .trim_end_matches('/')
            .to_lowercase()
    }
    #[cfg(not(windows))]
    {
        value.trim_end_matches('/').to_string()
    }
}

fn canonical_local_root(raw: &str) -> McpResult<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(err(
            McpErrorCode::ERR_INVALID_PATH,
            "local storage root must not be empty",
        ));
    }
    let path = Path::new(trimmed);
    if !path.is_absolute() {
        return Err(err(
            McpErrorCode::ERR_INVALID_PATH,
            "local storage root must be an absolute path",
        ));
    }
    if let Ok(canonical) = std::fs::canonicalize(path) {
        return Ok(canonical.to_string_lossy().to_string());
    }
    // Deterministic fallback when the root is temporarily unresolvable so the
    // fingerprint remains computable for update-safety checks.
    let mut fallback = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                fallback.pop();
            }
            other => fallback.push(other.as_os_str()),
        }
    }
    let mut result = fallback.to_string_lossy().to_string();
    if result.is_empty() {
        result = trimmed.to_string();
    }
    #[cfg(windows)]
    {
        result = result.to_lowercase();
    }
    Ok(result)
}

fn normalize_root_prefix(root: &str) -> String {
    let trimmed = root.trim().trim_matches('/');
    if trimmed.is_empty() {
        String::new()
    } else {
        format!("/{trimmed}")
    }
}

/// The public config used for identity: the full non-secret config with every
/// secret-classified scalar stripped. Conservative by design: a harmless public
/// config edit may change the fingerprint, but a namespace change can never be missed.
fn public_config_for_fingerprint(storage: &StorageRecord) -> McpResult<serde_json::Value> {
    let mut config = storage.config.clone();
    let schema_names = secrets::discover_secret_field_names();
    secrets::strip_secret_fields(&mut config, &schema_names);
    // Secret stripping can leave empty containers behind (nested-array secrets).
    // They carry no identity and must not make a secret-only edit look like a
    // namespace change.
    secrets::prune_empty_containers(&mut config);
    Ok(config)
}

fn canonical_json(value: &serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Object(map) => {
            let mut ordered = BTreeMap::new();
            for (key, child) in map {
                ordered.insert(key.clone(), canonical_json(child));
            }
            serde_json::Value::Object(ordered.into_iter().collect())
        }
        serde_json::Value::Array(items) => {
            serde_json::Value::Array(items.iter().map(canonical_json).collect())
        }
        other => other.clone(),
    }
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut output = String::with_capacity(digest.len() * 2);
    for byte in digest {
        output.push_str(&format!("{byte:02x}"));
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn storage(id: &str, backend: &str, config: serde_json::Value) -> StorageRecord {
        StorageRecord::new(id.to_string(), backend.to_string(), config)
    }

    #[test]
    fn deterministic_fingerprint_ignores_key_ordering() {
        let left = storage(
            "one",
            "s3",
            json!({ "bucket": "example", "region": "us-east-1" }),
        );
        let mut right = storage(
            "two",
            "s3",
            json!({ "region": "us-east-1", "bucket": "example" }),
        );
        right.config = canonical_json(&right.config);
        assert_eq!(
            storage_namespace_fingerprint(&left).unwrap(),
            storage_namespace_fingerprint(&right).unwrap()
        );
    }

    #[test]
    fn different_local_root_changes_fingerprint() {
        let temp = tempfile::tempdir().unwrap();
        let a = temp.path().join("root-a");
        let b = temp.path().join("root-b");
        std::fs::create_dir_all(&a).unwrap();
        std::fs::create_dir_all(&b).unwrap();
        let left = storage("a", "local", json!({ "root": a.to_string_lossy() }));
        let right = storage("b", "local", json!({ "root": b.to_string_lossy() }));
        assert_ne!(
            storage_namespace_fingerprint(&left).unwrap(),
            storage_namespace_fingerprint(&right).unwrap()
        );
    }

    #[test]
    fn secret_values_do_not_enter_fingerprint() {
        let clean = storage(
            "a",
            "s3",
            json!({ "bucket": "example", "region": "us-east-1" }),
        );
        let secret = storage(
            "b",
            "s3",
            json!({
                "bucket": "example",
                "region": "us-east-1",
                "accessKeyId": "AKIA-SECRET",
                "secretAccessKey": "super-secret"
            }),
        );
        assert_eq!(
            storage_namespace_fingerprint(&clean).unwrap(),
            storage_namespace_fingerprint(&secret).unwrap()
        );
    }

    #[test]
    fn same_local_path_through_two_ids_is_detected_as_same_namespace() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().to_string_lossy().to_string();
        let a = storage("one", "local", json!({ "root": root }));
        let b = storage("two", "local", json!({ "root": root }));
        assert_eq!(
            storage_namespace_fingerprint(&a).unwrap(),
            storage_namespace_fingerprint(&b).unwrap()
        );
        let relation = transfer_namespace_relation(&a, "foo", &b, "bar").unwrap();
        assert!(relation.same_underlying_namespace);
        assert!(!relation.same_operator_scope);
        assert!(!transfer_has_namespace_conflict(&relation));
    }

    #[test]
    fn descendant_alias_into_nested_root_is_rejected() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("root");
        let nested = root.join("foo");
        std::fs::create_dir_all(&nested).unwrap();
        let a = storage("one", "local", json!({ "root": root.to_string_lossy() }));
        let b = storage("two", "local", json!({ "root": nested.to_string_lossy() }));
        let relation = transfer_namespace_relation(&a, "foo", &b, "child").unwrap();
        assert!(relation.same_underlying_namespace);
        assert!(transfer_has_namespace_conflict(&relation));
    }

    #[test]
    fn copy_into_self_subdirectory_is_rejected() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().to_string_lossy().to_string();
        let a = storage("one", "local", json!({ "root": root }));
        let b = storage("two", "local", json!({ "root": root }));
        let relation = transfer_namespace_relation(&a, "foo", &b, "foo/child").unwrap();
        assert!(transfer_has_namespace_conflict(&relation));
    }

    #[test]
    fn unrelated_local_paths_do_not_conflict() {
        let temp = tempfile::tempdir().unwrap();
        let a = storage(
            "one",
            "local",
            json!({ "root": temp.path().join("a").to_string_lossy() }),
        );
        let b = storage(
            "two",
            "local",
            json!({ "root": temp.path().join("b").to_string_lossy() }),
        );
        std::fs::create_dir_all(temp.path().join("a")).unwrap();
        std::fs::create_dir_all(temp.path().join("b")).unwrap();
        let relation = transfer_namespace_relation(&a, "foo", &b, "bar").unwrap();
        assert!(relation.same_underlying_namespace);
        assert!(!transfer_has_namespace_conflict(&relation));
    }

    #[test]
    fn object_store_alias_through_same_endpoint_and_container_is_rejected() {
        let a = storage(
            "one",
            "s3",
            json!({ "bucket": "bucket", "endpoint": "https://s3.example.com" }),
        );
        let b = storage(
            "two",
            "s3",
            json!({ "bucket": "bucket", "endpoint": "https://S3.EXAMPLE.COM:443" }),
        );
        // The conservative public-config hash may differ on cosmetic endpoint
        // edits; the canonical alias detection must still agree.
        let relation = transfer_namespace_relation(&a, "foo", &b, "foo/child").unwrap();
        assert!(relation.same_underlying_namespace);
        assert!(transfer_has_namespace_conflict(&relation));
        let exact = storage(
            "three",
            "s3",
            json!({ "bucket": "bucket", "endpoint": "https://s3.example.com" }),
        );
        assert_eq!(
            storage_namespace_fingerprint(&a).unwrap(),
            storage_namespace_fingerprint(&exact).unwrap()
        );
    }

    #[test]
    fn different_object_store_buckets_do_not_conflict() {
        let a = storage(
            "one",
            "s3",
            json!({ "bucket": "bucket-a", "endpoint": "https://s3.example.com" }),
        );
        let b = storage(
            "two",
            "s3",
            json!({ "bucket": "bucket-b", "endpoint": "https://s3.example.com" }),
        );
        let relation = transfer_namespace_relation(&a, "foo", &b, "foo/child").unwrap();
        assert!(!relation.same_underlying_namespace);
        assert!(!transfer_has_namespace_conflict(&relation));
    }

    #[test]
    fn local_requires_absolute_root() {
        let local = storage("one", "local", json!({ "root": "relative/path" }));
        assert!(storage_namespace_fingerprint(&local).is_err());
    }

    #[test]
    fn operation_path_escape_is_rejected() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().to_string_lossy().to_string();
        let local = storage("one", "local", json!({ "root": root }));
        let result = storage_namespace_address(&local, "a/../../etc/passwd");
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().code, McpErrorCode::ERR_INVALID_PATH);
    }

    #[test]
    fn gdrive_different_secret_references_are_unknown() {
        let mut a = storage("one", "gdrive", json!({ "rootPath": "/workspace" }));
        let mut b = storage("two", "gdrive", json!({ "rootPath": "/workspace" }));
        a.secret_ref = Some("storage/one".to_string());
        b.secret_ref = Some("storage/two".to_string());
        let relation = transfer_namespace_relation(&a, "foo", &b, "foo/child").unwrap();
        assert!(!relation.same_underlying_namespace);
        assert!(!transfer_has_namespace_conflict(&relation));
    }

    #[test]
    fn s3_region_only_and_default_endpoint_are_the_same_namespace() {
        let region_only = storage(
            "one",
            "s3",
            json!({ "bucket": "bucket", "region": "eu-west-1" }),
        );
        let explicit_default = storage(
            "two",
            "s3",
            json!({ "bucket": "bucket", "region": "eu-west-1", "endpoint": "https://s3.eu-west-1.amazonaws.com" }),
        );
        let relation =
            transfer_namespace_relation(&region_only, "foo", &explicit_default, "foo/child")
                .unwrap();
        assert!(relation.same_underlying_namespace);
        assert!(transfer_has_namespace_conflict(&relation));
    }

    #[test]
    fn s3_default_region_with_and_without_endpoint_are_the_same_namespace() {
        let legacy_default = storage("one", "s3", json!({ "bucket": "bucket" }));
        let explicit_us_east_1 = storage(
            "two",
            "s3",
            json!({ "bucket": "bucket", "endpoint": "https://s3.amazonaws.com" }),
        );
        let relation =
            transfer_namespace_relation(&legacy_default, "foo", &explicit_us_east_1, "foo/child")
                .unwrap();
        assert!(relation.same_underlying_namespace);
        assert!(transfer_has_namespace_conflict(&relation));
    }

    #[test]
    fn azure_account_and_default_endpoint_are_the_same_namespace() {
        let account_only = storage(
            "one",
            "azblob",
            json!({ "accountName": "demoacct", "container": "bucket" }),
        );
        let explicit_default = storage(
            "two",
            "azblob",
            json!({ "accountName": "demoacct", "container": "bucket", "endpoint": "https://demoacct.blob.core.windows.net" }),
        );
        let relation =
            transfer_namespace_relation(&account_only, "foo", &explicit_default, "foo/child")
                .unwrap();
        assert!(relation.same_underlying_namespace);
        assert!(transfer_has_namespace_conflict(&relation));
    }

    #[test]
    fn azure_explicit_custom_endpoint_does_not_collapse_to_account() {
        let account_only = storage(
            "one",
            "azblob",
            json!({ "accountName": "demoacct", "container": "bucket" }),
        );
        let custom_endpoint = storage(
            "two",
            "azblob",
            json!({ "accountName": "demoacct", "container": "bucket", "endpoint": "https://storage.internal.example.com" }),
        );
        let relation =
            transfer_namespace_relation(&account_only, "foo", &custom_endpoint, "foo/child")
                .unwrap();
        assert!(!relation.same_underlying_namespace);
    }

    #[cfg(windows)]
    #[test]
    fn windows_local_descendant_uses_component_boundary() {
        let a = storage("one", "local", json!({ "root": "C:\\root" }));
        let b = storage("two", "local", json!({ "root": "C:\\root" }));
        // Backslash-separated roots combined with forward-slash backend paths
        // must still be detected as an equal/descendant conflict.
        let relation = transfer_namespace_relation(&a, "foo", &b, "foo/child").unwrap();
        assert!(relation.same_underlying_namespace);
        assert!(transfer_has_namespace_conflict(&relation));
        // A sibling whose name merely shares the prefix must not conflict.
        let sibling = transfer_namespace_relation(&a, "foo2", &b, "foo2").unwrap();
        assert!(!transfer_has_namespace_conflict(&sibling));
    }
}
