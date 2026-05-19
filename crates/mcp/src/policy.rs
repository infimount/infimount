use serde::{Deserialize, Serialize};

use crate::errors::{err_with_details, McpError, McpErrorCode};
use crate::registry::StorageRecord;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum McpAccessMode {
    None,
    ReadOnly,
    ReadWrite,
}

fn default_access_mode() -> McpAccessMode {
    McpAccessMode::ReadWrite
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct McpConfirmationRules {
    pub require_for_write: bool,
    pub require_for_overwrite: bool,
    pub require_for_delete: bool,
    pub require_for_version_delete: bool,
    pub require_for_presign: bool,
    pub require_for_cross_storage_copy: bool,
}

impl Default for McpConfirmationRules {
    fn default() -> Self {
        Self {
            require_for_write: true,
            require_for_overwrite: true,
            require_for_delete: true,
            require_for_version_delete: true,
            require_for_presign: true,
            require_for_cross_storage_copy: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct McpStoragePolicy {
    #[serde(default = "default_access_mode")]
    pub default_access: McpAccessMode,
    pub allowed_paths: Vec<String>,
    pub denied_paths: Vec<String>,
    pub confirmation_rules: McpConfirmationRules,
}

impl Default for McpStoragePolicy {
    fn default() -> Self {
        Self {
            default_access: default_access_mode(),
            allowed_paths: Vec::new(),
            denied_paths: Vec::new(),
            confirmation_rules: McpConfirmationRules::default(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum McpOperation {
    List,
    Read,
    Metadata,
    Search,
    Write,
    Upload,
    Mkdir,
    Copy,
    Move,
    Rename,
    Delete,
    ListVersions,
    ReadFileVersion,
    DeleteVersion,
    RestoreVersion,
    PresignDownloadLink,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum McpRiskType {
    Write,
    Overwrite,
    Delete,
    VersionDelete,
    PublicOrExternalLink,
    CrossStorageCopy,
    RenameCopyDelete,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PolicyDecision {
    Allow,
    RequireConfirmation { risk_type: McpRiskType },
}

impl McpOperation {
    pub fn is_write_like(self) -> bool {
        matches!(
            self,
            Self::Write
                | Self::Upload
                | Self::Mkdir
                | Self::Copy
                | Self::Move
                | Self::Rename
                | Self::Delete
                | Self::DeleteVersion
                | Self::RestoreVersion
        )
    }

    pub fn is_read_like(self) -> bool {
        matches!(
            self,
            Self::List
                | Self::Read
                | Self::Metadata
                | Self::Search
                | Self::ListVersions
                | Self::ReadFileVersion
                | Self::PresignDownloadLink
        )
    }
}

pub fn evaluate_storage_policy(
    storage: &StorageRecord,
    backend_path: &str,
    operation: McpOperation,
    overwrite: bool,
    cross_storage: bool,
) -> Result<PolicyDecision, McpError> {
    if !storage.mcp_exposed {
        return Err(err_with_details(
            McpErrorCode::ERR_STORAGE_NOT_EXPOSED,
            format!("Storage '{}' is not exposed to MCP", storage.name),
            serde_json::json!({ "storage_name": storage.name }),
        ));
    }

    let normalized_path = normalize_policy_path(backend_path);
    let policy = &storage.mcp_policy;

    if policy
        .denied_paths
        .iter()
        .any(|prefix| path_matches_prefix(&normalized_path, prefix))
    {
        return Err(policy_denied(
            storage,
            &normalized_path,
            "path denied by MCP policy",
        ));
    }

    if !policy.allowed_paths.is_empty()
        && !policy
            .allowed_paths
            .iter()
            .any(|prefix| path_matches_prefix(&normalized_path, prefix))
    {
        return Err(policy_denied(
            storage,
            &normalized_path,
            "path is outside allowed MCP prefixes",
        ));
    }

    if storage.read_only && operation.is_write_like() {
        return Err(err_with_details(
            McpErrorCode::ERR_STORAGE_READ_ONLY,
            format!("Storage '{}' is read-only", storage.name),
            serde_json::json!({ "storage_name": storage.name, "path": normalized_path }),
        ));
    }

    match policy.default_access {
        McpAccessMode::None => Err(policy_denied(
            storage,
            &normalized_path,
            "MCP access is disabled by storage policy",
        )),
        McpAccessMode::ReadOnly if operation.is_write_like() => Err(policy_denied(
            storage,
            &normalized_path,
            "MCP policy is read-only for this storage",
        )),
        McpAccessMode::ReadOnly | McpAccessMode::ReadWrite => {
            let rules = &policy.confirmation_rules;
            let risk_type = if cross_storage && rules.require_for_cross_storage_copy {
                Some(McpRiskType::CrossStorageCopy)
            } else if matches!(operation, McpOperation::DeleteVersion)
                && rules.require_for_version_delete
            {
                Some(McpRiskType::VersionDelete)
            } else if matches!(operation, McpOperation::Delete) && rules.require_for_delete {
                Some(McpRiskType::Delete)
            } else if matches!(operation, McpOperation::PresignDownloadLink)
                && rules.require_for_presign
            {
                Some(McpRiskType::PublicOrExternalLink)
            } else if matches!(operation, McpOperation::Move | McpOperation::Rename)
                && rules.require_for_delete
            {
                Some(McpRiskType::RenameCopyDelete)
            } else if overwrite && rules.require_for_overwrite {
                Some(McpRiskType::Overwrite)
            } else if operation.is_write_like() && rules.require_for_write {
                Some(McpRiskType::Write)
            } else {
                None
            };

            if let Some(risk_type) = risk_type {
                Ok(PolicyDecision::RequireConfirmation { risk_type })
            } else {
                Ok(PolicyDecision::Allow)
            }
        }
    }
}

fn policy_denied(storage: &StorageRecord, path: &str, message: &str) -> McpError {
    err_with_details(
        McpErrorCode::ERR_MCP_POLICY_DENIED,
        message,
        serde_json::json!({
            "storage_id": storage.id,
            "storage_name": storage.name,
            "path": path
        }),
    )
}

pub fn normalize_policy_path(path: &str) -> String {
    let decoded = decode_policy_path_controls(path.trim());
    let mut segments = Vec::new();

    for segment in decoded.split('/') {
        match segment {
            "" | "." => {}
            ".." => {
                segments.pop();
            }
            value => segments.push(value),
        }
    }

    segments.join("/")
}

fn decode_policy_path_controls(path: &str) -> String {
    let mut out = String::with_capacity(path.len());
    let bytes = path.as_bytes();
    let mut index = 0;

    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            let code = &path[index + 1..index + 3].to_ascii_lowercase();
            match code.as_str() {
                "2f" | "5c" => {
                    out.push('/');
                    index += 3;
                    continue;
                }
                "2e" => {
                    out.push('.');
                    index += 3;
                    continue;
                }
                _ => {}
            }
        }

        if bytes[index] == b'\\' {
            out.push('/');
        } else {
            out.push(bytes[index] as char);
        }
        index += 1;
    }

    out
}

fn path_matches_prefix(path: &str, prefix: &str) -> bool {
    let prefix = normalize_policy_path(prefix);
    if prefix.is_empty() {
        return true;
    }

    path == prefix || path.strip_prefix(&(prefix + "/")).is_some()
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::registry::StorageRecord;

    fn storage_with_policy(policy: McpStoragePolicy) -> StorageRecord {
        let mut storage = StorageRecord::new(
            "Local".to_string(),
            "local".to_string(),
            json!({ "root": "/tmp" }),
        );
        storage.mcp_policy = policy;
        storage
    }

    #[test]
    fn deny_prefix_wins_over_allowed_prefix() {
        let storage = storage_with_policy(McpStoragePolicy {
            allowed_paths: vec!["projects".to_string()],
            denied_paths: vec!["projects/secrets".to_string()],
            ..Default::default()
        });

        let error = evaluate_storage_policy(
            &storage,
            "projects/secrets/token.txt",
            McpOperation::Read,
            false,
            false,
        )
        .unwrap_err();

        assert_eq!(error.code, McpErrorCode::ERR_MCP_POLICY_DENIED);
    }

    #[test]
    fn allowed_prefix_is_segment_aware() {
        let storage = storage_with_policy(McpStoragePolicy {
            allowed_paths: vec!["foo".to_string()],
            ..Default::default()
        });

        assert!(
            evaluate_storage_policy(&storage, "foo/a.txt", McpOperation::Read, false, false)
                .is_ok()
        );
        assert!(evaluate_storage_policy(
            &storage,
            "foobar/a.txt",
            McpOperation::Read,
            false,
            false
        )
        .is_err());
    }

    #[test]
    fn read_only_policy_blocks_writes() {
        let storage = storage_with_policy(McpStoragePolicy {
            default_access: McpAccessMode::ReadOnly,
            ..Default::default()
        });

        let error = evaluate_storage_policy(&storage, "a.txt", McpOperation::Write, false, false)
            .unwrap_err();

        assert_eq!(error.code, McpErrorCode::ERR_MCP_POLICY_DENIED);
    }

    #[test]
    fn risky_delete_requires_confirmation_by_default() {
        let storage = storage_with_policy(McpStoragePolicy::default());
        let decision =
            evaluate_storage_policy(&storage, "a.txt", McpOperation::Delete, false, false).unwrap();

        assert_eq!(
            decision,
            PolicyDecision::RequireConfirmation {
                risk_type: McpRiskType::Delete
            }
        );
    }

    #[test]
    fn disabled_confirmation_allows_matching_read_write_operation() {
        let storage = storage_with_policy(McpStoragePolicy {
            confirmation_rules: McpConfirmationRules {
                require_for_write: false,
                require_for_overwrite: false,
                ..Default::default()
            },
            ..Default::default()
        });

        let decision =
            evaluate_storage_policy(&storage, "a.txt", McpOperation::Write, false, false).unwrap();

        assert_eq!(decision, PolicyDecision::Allow);
    }

    #[test]
    fn path_policy_normalizes_bypass_attempts_before_matching_prefixes() {
        let storage = storage_with_policy(McpStoragePolicy {
            allowed_paths: vec!["projects".to_string()],
            denied_paths: vec!["projects/secrets".to_string()],
            ..Default::default()
        });

        for path in [
            "/projects//secrets/token.txt",
            "projects/secrets/",
            "projects/./secrets/token.txt",
            "projects/public/../secrets/token.txt",
            "projects/public/%2e%2e/secrets/token.txt",
            "projects%2fsecrets/token.txt",
        ] {
            let error = evaluate_storage_policy(&storage, path, McpOperation::Read, false, false)
                .unwrap_err();
            assert_eq!(error.code, McpErrorCode::ERR_MCP_POLICY_DENIED, "{path}");
        }
    }

    #[test]
    fn path_policy_remains_case_sensitive() {
        let storage = storage_with_policy(McpStoragePolicy {
            denied_paths: vec!["Secrets".to_string()],
            ..Default::default()
        });

        assert!(evaluate_storage_policy(
            &storage,
            "secrets/token.txt",
            McpOperation::Read,
            false,
            false
        )
        .is_ok());
        assert!(evaluate_storage_policy(
            &storage,
            "Secrets/token.txt",
            McpOperation::Read,
            false,
            false
        )
        .is_err());
    }

    #[test]
    fn mcp_disabled_storage_blocks_every_operation() {
        let mut storage = storage_with_policy(McpStoragePolicy::default());
        storage.mcp_exposed = false;

        for operation in [
            McpOperation::List,
            McpOperation::Read,
            McpOperation::Metadata,
            McpOperation::Search,
            McpOperation::Write,
            McpOperation::Mkdir,
            McpOperation::Copy,
            McpOperation::Move,
            McpOperation::Delete,
            McpOperation::PresignDownloadLink,
            McpOperation::DeleteVersion,
        ] {
            let error =
                evaluate_storage_policy(&storage, "a.txt", operation, false, false).unwrap_err();
            assert_eq!(error.code, McpErrorCode::ERR_STORAGE_NOT_EXPOSED);
        }
    }

    #[test]
    fn read_only_storage_blocks_write_like_operations() {
        let mut storage = storage_with_policy(McpStoragePolicy::default());
        storage.read_only = true;

        for operation in [
            McpOperation::Write,
            McpOperation::Upload,
            McpOperation::Mkdir,
            McpOperation::Copy,
            McpOperation::Move,
            McpOperation::Rename,
            McpOperation::Delete,
            McpOperation::DeleteVersion,
            McpOperation::RestoreVersion,
        ] {
            let error =
                evaluate_storage_policy(&storage, "a.txt", operation, false, false).unwrap_err();
            assert_eq!(error.code, McpErrorCode::ERR_STORAGE_READ_ONLY);
        }
    }

    #[test]
    fn risky_operations_have_specific_confirmation_classification() {
        let storage = storage_with_policy(McpStoragePolicy::default());

        assert_eq!(
            evaluate_storage_policy(&storage, "a.txt", McpOperation::Move, false, false).unwrap(),
            PolicyDecision::RequireConfirmation {
                risk_type: McpRiskType::RenameCopyDelete
            }
        );
        assert_eq!(
            evaluate_storage_policy(&storage, "a.txt", McpOperation::Copy, false, true).unwrap(),
            PolicyDecision::RequireConfirmation {
                risk_type: McpRiskType::CrossStorageCopy
            }
        );
        assert_eq!(
            evaluate_storage_policy(
                &storage,
                "a.txt",
                McpOperation::PresignDownloadLink,
                false,
                false
            )
            .unwrap(),
            PolicyDecision::RequireConfirmation {
                risk_type: McpRiskType::PublicOrExternalLink
            }
        );
        assert_eq!(
            evaluate_storage_policy(&storage, "a.txt", McpOperation::DeleteVersion, false, false)
                .unwrap(),
            PolicyDecision::RequireConfirmation {
                risk_type: McpRiskType::VersionDelete
            }
        );
        assert_eq!(
            evaluate_storage_policy(&storage, "a.txt", McpOperation::RestoreVersion, true, false)
                .unwrap(),
            PolicyDecision::RequireConfirmation {
                risk_type: McpRiskType::Overwrite
            }
        );
    }
}
