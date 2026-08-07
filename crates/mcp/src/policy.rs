use serde::{Deserialize, Serialize};

use crate::errors::{err, err_with_details, McpError, McpErrorCode};
use crate::registry::StorageRecord;

pub const MCP_POLICY_VERSION: u32 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum McpAccessMode {
    None,
    ReadOnly,
    ReadWrite,
}

fn none_access_mode() -> McpAccessMode {
    McpAccessMode::None
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
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum McpRuleSource {
    Manual,
    Workspace { workspace_id: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpPathRule {
    pub id: String,
    pub prefix: String,
    pub access: McpAccessMode,
    pub source: McpRuleSource,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confirmation_rules: Option<McpConfirmationRules>,
}

fn default_policy_version() -> u32 {
    0
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpStoragePolicy {
    #[serde(default = "default_policy_version")]
    pub version: u32,
    #[serde(default = "none_access_mode")]
    pub default_access: McpAccessMode,
    #[serde(default)]
    pub rules: Vec<McpPathRule>,
    #[serde(default)]
    pub denied_paths: Vec<String>,
    #[serde(default)]
    pub confirmation_rules: McpConfirmationRules,

    #[serde(default, skip_serializing)]
    pub allowed_paths: Vec<String>,
}

impl Default for McpStoragePolicy {
    fn default() -> Self {
        Self {
            version: MCP_POLICY_VERSION,
            default_access: none_access_mode(),
            rules: Vec::new(),
            denied_paths: Vec::new(),
            confirmation_rules: McpConfirmationRules::default(),
            allowed_paths: Vec::new(),
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum PolicyDecision {
    Allow,
    RequireConfirmation { risk_type: McpRiskType },
}

#[derive(Debug, Clone, Serialize)]
pub struct PolicyEvaluation {
    pub decision: PolicyDecision,
    pub matched_rule_id: Option<String>,
    pub workspace_id: Option<String>,
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

pub fn normalize_policy_path(path: &str) -> Result<String, McpError> {
    let decoded = decode_policy_path_controls(path.trim());
    let mut segments: Vec<String> = Vec::new();

    for segment in decoded.split('/') {
        match segment {
            "" | "." => {}
            ".." => {
                segments.pop();
            }
            value => segments.push(value.to_string()),
        }
    }

    let normalized = segments.join("/");
    if normalized.starts_with('/') || normalized.ends_with('/') {
        // Should not happen after above processing, but safeguard
        return Err(err_with_details(
            McpErrorCode::ERR_INVALID_PATH,
            "normalized path must not start or end with '/'",
            serde_json::json!({ "path": path, "normalized": normalized }),
        ));
    }

    Ok(normalized)
}

fn decode_policy_path_controls(path: &str) -> String {
    let mut out = String::with_capacity(path.len());
    let bytes = path.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            let a = bytes[index + 1].to_ascii_lowercase();
            let b = bytes[index + 2].to_ascii_lowercase();
            let decoded = match (a, b) {
                (b'2', b'f') | (b'5', b'c') => Some('/'),
                (b'2', b'e') => Some('.'),
                _ => None,
            };
            if let Some(decoded) = decoded {
                out.push(decoded);
                index += 3;
                continue;
            }
        }
        let Some(ch) = path[index..].chars().next() else {
            break;
        };
        if ch == '\\' {
            out.push('/');
        } else if !ch.is_control() || matches!(ch, '\n' | '\r' | '\t') {
            out.push(ch);
        }
        index += ch.len_utf8();
    }
    out
}

fn path_matches_normalized_prefix(path: &str, prefix: &str) -> bool {
    prefix.is_empty()
        || path == prefix
        || path
            .strip_prefix(prefix)
            .is_some_and(|suffix| suffix.starts_with('/'))
}

pub fn normalize_policy_rule(rule: &mut McpPathRule) -> Result<(), McpError> {
    rule.prefix = normalize_policy_path(&rule.prefix)?;
    Ok(())
}

pub fn normalize_storage_policy(policy: &mut McpStoragePolicy) -> Result<(), McpError> {
    let mut normalized_denied = Vec::new();
    for p in &policy.denied_paths {
        normalized_denied.push(normalize_policy_path(p)?);
    }
    policy.denied_paths = normalized_denied;
    for rule in &mut policy.rules {
        normalize_policy_rule(rule)?;
        if rule.prefix.is_empty() {
            return Err(err_with_details(
                McpErrorCode::ERR_INVALID_PATH,
                "policy rules cannot grant the storage root",
                serde_json::json!({ "rule_id": rule.id }),
            ));
        }
    }

    let mut seen_rule_ids = std::collections::HashSet::new();
    let mut seen_prefixes = std::collections::HashSet::new();
    for rule in &policy.rules {
        if !seen_rule_ids.insert(&rule.id) {
            return Err(err_with_details(
                McpErrorCode::ERR_INVALID_POLICY,
                format!("duplicate policy rule ID '{}'", rule.id),
                serde_json::json!({ "rule_id": rule.id }),
            ));
        }
        if !seen_prefixes.insert(&rule.prefix) {
            return Err(err_with_details(
                McpErrorCode::ERR_INVALID_POLICY,
                format!(
                    "duplicate normalized rule prefix '{}' (rule '{}')",
                    rule.prefix, rule.id
                ),
                serde_json::json!({
                    "prefix": rule.prefix,
                    "rule_id": rule.id,
                }),
            ));
        }
    }
    Ok(())
}

fn validate_persisted_policy(policy: &McpStoragePolicy) -> Result<(), McpError> {
    let mut seen_rule_ids = std::collections::HashSet::with_capacity(policy.rules.len());
    let mut seen_prefixes = std::collections::HashSet::with_capacity(policy.rules.len());
    for rule in &policy.rules {
        if !seen_rule_ids.insert(&rule.id) {
            return Err(err_with_details(
                McpErrorCode::ERR_INVALID_POLICY,
                "storage policy contains a duplicate persisted rule ID",
                serde_json::json!({ "rule_id": rule.id }),
            ));
        }
        let normalized = normalize_policy_path(&rule.prefix).map_err(|_| {
            err_with_details(
                McpErrorCode::ERR_INVALID_POLICY,
                "storage policy contains an invalid persisted rule prefix",
                serde_json::json!({ "rule_id": rule.id }),
            )
        })?;
        if normalized != rule.prefix
            || rule.prefix.is_empty()
            || !seen_prefixes.insert(&rule.prefix)
        {
            return Err(err_with_details(
                McpErrorCode::ERR_INVALID_POLICY,
                "storage policy contains a malformed or duplicate persisted rule prefix",
                serde_json::json!({ "rule_id": rule.id }),
            ));
        }
    }
    for prefix in &policy.denied_paths {
        let normalized = normalize_policy_path(prefix).map_err(|_| {
            err(
                McpErrorCode::ERR_INVALID_POLICY,
                "storage policy contains an invalid denied prefix",
            )
        })?;
        if normalized != *prefix {
            return Err(err(
                McpErrorCode::ERR_INVALID_POLICY,
                "storage policy contains a non-normalized denied prefix",
            ));
        }
    }
    Ok(())
}

fn find_matching_rule<'a>(
    rules: &'a [McpPathRule],
    normalized_path: &str,
) -> Option<&'a McpPathRule> {
    rules
        .iter()
        .filter(|rule| path_matches_normalized_prefix(normalized_path, &rule.prefix))
        .max_by_key(|rule| rule.prefix.len())
}

pub fn migrate_legacy_policy(policy: &mut McpStoragePolicy) -> Result<(), McpError> {
    if policy.version == MCP_POLICY_VERSION {
        return Ok(());
    }

    if !policy.allowed_paths.is_empty() {
        let mut new_rules: Vec<McpPathRule> = Vec::new();
        for (i, path) in policy.allowed_paths.iter().enumerate() {
            let access = policy.default_access;
            let migrated_access = match access {
                McpAccessMode::ReadWrite => McpAccessMode::ReadWrite,
                McpAccessMode::ReadOnly => McpAccessMode::ReadOnly,
                McpAccessMode::None => McpAccessMode::ReadOnly,
            };
            new_rules.push(McpPathRule {
                id: format!("migrated-{}", i),
                prefix: path.clone(),
                access: migrated_access,
                source: McpRuleSource::Manual,
                confirmation_rules: None,
            });
        }
        policy.rules = new_rules;
        policy.default_access = McpAccessMode::None;
    }

    policy.version = MCP_POLICY_VERSION;
    policy.allowed_paths.clear();
    normalize_storage_policy(policy)
}

pub fn evaluate_storage_policy(
    storage: &StorageRecord,
    backend_path: &str,
    operation: McpOperation,
    overwrite: bool,
    cross_storage: bool,
) -> Result<PolicyEvaluation, McpError> {
    if !storage.mcp_exposed {
        return Err(err_with_details(
            McpErrorCode::ERR_STORAGE_NOT_EXPOSED,
            format!("Storage '{}' is not exposed to MCP", storage.name),
            serde_json::json!({ "storage_name": storage.name }),
        ));
    }

    let normalized_path = normalize_policy_path(backend_path)?;
    let policy = &storage.mcp_policy;

    // Registry persistence normalizes policies once. This linear validation keeps
    // evaluation fail-closed for legacy/corrupt files without O(n²) duplicate scans.
    validate_persisted_policy(policy)?;

    if policy
        .denied_paths
        .iter()
        .any(|prefix| path_matches_normalized_prefix(&normalized_path, prefix))
    {
        return Err(policy_denied(
            storage,
            &normalized_path,
            "path denied by MCP policy",
            None,
            None,
        ));
    }

    let matched_rule = find_matching_rule(&policy.rules, &normalized_path);

    let (effective_access, matched_rule_id, workspace_id) = if let Some(rule) = matched_rule {
        let ws_id = match &rule.source {
            McpRuleSource::Workspace { workspace_id } => Some(workspace_id.clone()),
            McpRuleSource::Manual => None,
        };
        (rule.access, Some(rule.id.clone()), ws_id)
    } else {
        (policy.default_access, None, None)
    };

    if storage.read_only && operation.is_write_like() {
        return Err(err_with_details(
            McpErrorCode::ERR_STORAGE_READ_ONLY,
            format!("Storage '{}' is read-only", storage.name),
            serde_json::json!({
                "storage_name": storage.name,
                "path": normalized_path,
                "matched_rule_id": matched_rule_id,
                "workspace_id": workspace_id
            }),
        ));
    }

    match effective_access {
        McpAccessMode::None => Err(policy_denied(
            storage,
            &normalized_path,
            "MCP access is disabled by storage policy",
            matched_rule_id.as_deref(),
            workspace_id.as_deref(),
        )),
        McpAccessMode::ReadOnly if operation.is_write_like() => Err(policy_denied(
            storage,
            &normalized_path,
            "MCP policy is read-only for this storage",
            matched_rule_id.as_deref(),
            workspace_id.as_deref(),
        )),
        McpAccessMode::ReadOnly | McpAccessMode::ReadWrite => {
            let rules = matched_rule
                .and_then(|r| r.confirmation_rules.as_ref())
                .unwrap_or(&policy.confirmation_rules);
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

            let decision = if let Some(risk_type) = risk_type {
                PolicyDecision::RequireConfirmation { risk_type }
            } else {
                PolicyDecision::Allow
            };

            Ok(PolicyEvaluation {
                decision,
                matched_rule_id,
                workspace_id,
            })
        }
    }
}

fn policy_denied(
    storage: &StorageRecord,
    path: &str,
    message: &str,
    matched_rule_id: Option<&str>,
    workspace_id: Option<&str>,
) -> McpError {
    err_with_details(
        McpErrorCode::ERR_MCP_POLICY_DENIED,
        message,
        serde_json::json!({
            "storage_id": storage.id,
            "storage_name": storage.name,
            "path": path,
            "matched_rule_id": matched_rule_id,
            "workspace_id": workspace_id
        }),
    )
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::registry::StorageRecord;

    fn storage_with_policy(policy: McpStoragePolicy) -> StorageRecord {
        let mut record = StorageRecord::new(
            "Local".to_string(),
            "local".to_string(),
            json!({ "root": "/tmp" }),
        );
        record.mcp_exposed = true;
        record.mcp_policy = policy;
        record
    }

    #[test]
    fn deny_prefix_wins_over_allowed_prefix() {
        let storage = storage_with_policy(McpStoragePolicy {
            rules: vec![McpPathRule {
                id: "allow-projects".to_string(),
                prefix: "projects".to_string(),
                access: McpAccessMode::ReadWrite,
                source: McpRuleSource::Manual,
                confirmation_rules: None,
            }],
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
            rules: vec![McpPathRule {
                id: "allow-foo".to_string(),
                prefix: "foo".to_string(),
                access: McpAccessMode::ReadWrite,
                source: McpRuleSource::Manual,
                confirmation_rules: None,
            }],
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
        let storage = storage_with_policy(McpStoragePolicy {
            default_access: McpAccessMode::ReadWrite,
            ..Default::default()
        });
        let eval =
            evaluate_storage_policy(&storage, "a.txt", McpOperation::Delete, false, false).unwrap();

        assert_eq!(
            eval.decision,
            PolicyDecision::RequireConfirmation {
                risk_type: McpRiskType::Delete
            }
        );
        assert_eq!(eval.matched_rule_id, None);
        assert_eq!(eval.workspace_id, None);
    }

    #[test]
    fn disabled_confirmation_allows_matching_read_write_operation() {
        let storage = storage_with_policy(McpStoragePolicy {
            default_access: McpAccessMode::ReadWrite,
            confirmation_rules: McpConfirmationRules {
                require_for_write: false,
                require_for_overwrite: false,
                ..Default::default()
            },
            ..Default::default()
        });

        let eval =
            evaluate_storage_policy(&storage, "a.txt", McpOperation::Write, false, false).unwrap();

        assert_eq!(eval.decision, PolicyDecision::Allow);
    }

    #[test]
    fn path_policy_normalizes_bypass_attempts_before_matching_prefixes() {
        let storage = storage_with_policy(McpStoragePolicy {
            rules: vec![McpPathRule {
                id: "allow-projects".to_string(),
                prefix: "projects".to_string(),
                access: McpAccessMode::ReadWrite,
                source: McpRuleSource::Manual,
                confirmation_rules: None,
            }],
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
            default_access: McpAccessMode::ReadWrite,
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
        let storage = storage_with_policy(McpStoragePolicy {
            default_access: McpAccessMode::ReadWrite,
            ..Default::default()
        });

        let eval =
            evaluate_storage_policy(&storage, "a.txt", McpOperation::Move, false, false).unwrap();
        assert_eq!(
            eval.decision,
            PolicyDecision::RequireConfirmation {
                risk_type: McpRiskType::RenameCopyDelete
            }
        );

        let eval =
            evaluate_storage_policy(&storage, "a.txt", McpOperation::Copy, false, true).unwrap();
        assert_eq!(
            eval.decision,
            PolicyDecision::RequireConfirmation {
                risk_type: McpRiskType::CrossStorageCopy
            }
        );

        let eval = evaluate_storage_policy(
            &storage,
            "a.txt",
            McpOperation::PresignDownloadLink,
            false,
            false,
        )
        .unwrap();
        assert_eq!(
            eval.decision,
            PolicyDecision::RequireConfirmation {
                risk_type: McpRiskType::PublicOrExternalLink
            }
        );

        let eval =
            evaluate_storage_policy(&storage, "a.txt", McpOperation::DeleteVersion, false, false)
                .unwrap();
        assert_eq!(
            eval.decision,
            PolicyDecision::RequireConfirmation {
                risk_type: McpRiskType::VersionDelete
            }
        );

        let eval =
            evaluate_storage_policy(&storage, "a.txt", McpOperation::RestoreVersion, true, false)
                .unwrap();
        assert_eq!(
            eval.decision,
            PolicyDecision::RequireConfirmation {
                risk_type: McpRiskType::Overwrite
            }
        );
    }

    #[test]
    fn longest_prefix_match_wins() {
        let storage = storage_with_policy(McpStoragePolicy {
            rules: vec![
                McpPathRule {
                    id: "broad".to_string(),
                    prefix: "projects".to_string(),
                    access: McpAccessMode::ReadOnly,
                    source: McpRuleSource::Manual,
                    confirmation_rules: None,
                },
                McpPathRule {
                    id: "specific".to_string(),
                    prefix: "projects/myapp".to_string(),
                    access: McpAccessMode::ReadWrite,
                    source: McpRuleSource::Manual,
                    confirmation_rules: None,
                },
            ],
            default_access: McpAccessMode::None,
            ..Default::default()
        });

        let eval = evaluate_storage_policy(
            &storage,
            "projects/myapp/src/main.rs",
            McpOperation::Write,
            false,
            false,
        )
        .unwrap();
        assert_eq!(eval.matched_rule_id, Some("specific".to_string()));
        assert!(matches!(
            eval.decision,
            PolicyDecision::RequireConfirmation { .. }
        ));

        let eval = evaluate_storage_policy(
            &storage,
            "projects/other/file.txt",
            McpOperation::Write,
            false,
            false,
        )
        .unwrap_err();
        assert_eq!(eval.code, McpErrorCode::ERR_MCP_POLICY_DENIED);
    }

    #[test]
    fn workspace_rule_returns_workspace_id_in_evaluation() {
        let storage = storage_with_policy(McpStoragePolicy {
            rules: vec![McpPathRule {
                id: "workspace:w-1".to_string(),
                prefix: "agent-workspaces/coding".to_string(),
                access: McpAccessMode::ReadOnly,
                source: McpRuleSource::Workspace {
                    workspace_id: "w-1".to_string(),
                },
                confirmation_rules: None,
            }],
            default_access: McpAccessMode::None,
            ..Default::default()
        });

        let eval = evaluate_storage_policy(
            &storage,
            "agent-workspaces/coding/main.rs",
            McpOperation::Read,
            false,
            false,
        )
        .unwrap();

        assert_eq!(eval.matched_rule_id, Some("workspace:w-1".to_string()));
        assert_eq!(eval.workspace_id, Some("w-1".to_string()));
        assert_eq!(eval.decision, PolicyDecision::Allow);
    }

    #[test]
    fn workspace_read_only_denies_write() {
        let storage = storage_with_policy(McpStoragePolicy {
            rules: vec![McpPathRule {
                id: "workspace:w-2".to_string(),
                prefix: "workspace-root".to_string(),
                access: McpAccessMode::ReadOnly,
                source: McpRuleSource::Workspace {
                    workspace_id: "w-2".to_string(),
                },
                confirmation_rules: None,
            }],
            default_access: McpAccessMode::None,
            ..Default::default()
        });

        let error = evaluate_storage_policy(
            &storage,
            "workspace-root/new.txt",
            McpOperation::Write,
            false,
            false,
        )
        .unwrap_err();
        assert_eq!(error.code, McpErrorCode::ERR_MCP_POLICY_DENIED);
    }

    #[test]
    fn read_write_workspace_requires_confirmation_for_write() {
        let storage = storage_with_policy(McpStoragePolicy {
            rules: vec![McpPathRule {
                id: "workspace:w-write".to_string(),
                prefix: "workspace-root".to_string(),
                access: McpAccessMode::ReadWrite,
                source: McpRuleSource::Workspace {
                    workspace_id: "w-write".to_string(),
                },
                confirmation_rules: None,
            }],
            default_access: McpAccessMode::None,
            ..Default::default()
        });

        let evaluation = evaluate_storage_policy(
            &storage,
            "workspace-root/new.txt",
            McpOperation::Write,
            false,
            false,
        )
        .expect("workspace write should reach confirmation");
        assert!(matches!(
            evaluation.decision,
            PolicyDecision::RequireConfirmation {
                risk_type: McpRiskType::Write
            }
        ));
        assert_eq!(evaluation.workspace_id.as_deref(), Some("w-write"));
    }

    #[test]
    fn read_outside_workspace_denied_when_default_none() {
        let storage = storage_with_policy(McpStoragePolicy {
            rules: vec![McpPathRule {
                id: "workspace:w-3".to_string(),
                prefix: "workspace-root".to_string(),
                access: McpAccessMode::ReadOnly,
                source: McpRuleSource::Workspace {
                    workspace_id: "w-3".to_string(),
                },
                confirmation_rules: None,
            }],
            default_access: McpAccessMode::None,
            ..Default::default()
        });

        let error = evaluate_storage_policy(
            &storage,
            "outside/secret.txt",
            McpOperation::Read,
            false,
            false,
        )
        .unwrap_err();
        assert_eq!(error.code, McpErrorCode::ERR_MCP_POLICY_DENIED);
    }

    #[test]
    fn duplicate_normalized_prefixes_rejected() {
        let storage = storage_with_policy(McpStoragePolicy {
            rules: vec![
                McpPathRule {
                    id: "rule-a".to_string(),
                    prefix: "projects".to_string(),
                    access: McpAccessMode::ReadOnly,
                    source: McpRuleSource::Manual,
                    confirmation_rules: None,
                },
                McpPathRule {
                    id: "rule-b".to_string(),
                    prefix: "projects/".to_string(),
                    access: McpAccessMode::ReadWrite,
                    source: McpRuleSource::Manual,
                    confirmation_rules: None,
                },
            ],
            default_access: McpAccessMode::None,
            ..Default::default()
        });

        let error = evaluate_storage_policy(
            &storage,
            "projects/file.txt",
            McpOperation::Read,
            false,
            false,
        )
        .unwrap_err();
        assert_eq!(error.code, McpErrorCode::ERR_INVALID_POLICY);
    }

    #[test]
    fn global_deny_overrides_workspace_grant() {
        let storage = storage_with_policy(McpStoragePolicy {
            rules: vec![McpPathRule {
                id: "workspace:w-4".to_string(),
                prefix: "projects".to_string(),
                access: McpAccessMode::ReadWrite,
                source: McpRuleSource::Workspace {
                    workspace_id: "w-4".to_string(),
                },
                confirmation_rules: None,
            }],
            denied_paths: vec!["projects".to_string()],
            default_access: McpAccessMode::None,
            ..Default::default()
        });

        let error = evaluate_storage_policy(
            &storage,
            "projects/file.txt",
            McpOperation::Read,
            false,
            false,
        )
        .unwrap_err();
        assert_eq!(error.code, McpErrorCode::ERR_MCP_POLICY_DENIED);
    }

    #[test]
    fn default_policy_v2_grants_no_access() {
        let storage = storage_with_policy(McpStoragePolicy::default());
        assert_eq!(storage.mcp_policy.version, 2);
        assert_eq!(storage.mcp_policy.default_access, McpAccessMode::None);

        let error = evaluate_storage_policy(&storage, "any/path", McpOperation::Read, false, false)
            .unwrap_err();
        assert_eq!(error.code, McpErrorCode::ERR_MCP_POLICY_DENIED);
    }

    #[test]
    fn migrate_legacy_empty_allowed_paths_preserves_default() {
        let mut policy = McpStoragePolicy {
            version: 1,
            default_access: McpAccessMode::ReadOnly,
            rules: vec![],
            denied_paths: vec![],
            confirmation_rules: McpConfirmationRules::default(),
            allowed_paths: vec![],
        };

        migrate_legacy_policy(&mut policy).unwrap();

        assert_eq!(policy.version, 2);
        assert_eq!(policy.default_access, McpAccessMode::ReadOnly);
        assert!(policy.rules.is_empty());
    }

    #[test]
    fn migrate_legacy_read_write_with_allowed_paths() {
        let mut policy = McpStoragePolicy {
            version: 1,
            default_access: McpAccessMode::ReadWrite,
            rules: vec![],
            denied_paths: vec!["secret".to_string()],
            confirmation_rules: McpConfirmationRules::default(),
            allowed_paths: vec!["docs".to_string(), "projects".to_string()],
        };

        migrate_legacy_policy(&mut policy).unwrap();

        assert_eq!(policy.version, 2);
        assert_eq!(policy.default_access, McpAccessMode::None);
        assert_eq!(policy.rules.len(), 2);
        assert_eq!(policy.rules[0].access, McpAccessMode::ReadWrite);
        assert_eq!(policy.rules[0].prefix, "docs");
        assert_eq!(policy.rules[1].access, McpAccessMode::ReadWrite);
        assert_eq!(policy.rules[1].prefix, "projects");
        assert!(policy.allowed_paths.is_empty());
        assert_eq!(policy.denied_paths, vec!["secret"]);
    }

    #[test]
    fn migrate_legacy_none_with_allowed_path_repairs_to_read_only() {
        let mut policy = McpStoragePolicy {
            version: 1,
            default_access: McpAccessMode::None,
            rules: vec![],
            denied_paths: vec![],
            confirmation_rules: McpConfirmationRules::default(),
            allowed_paths: vec!["buggy-workspace".to_string()],
        };

        migrate_legacy_policy(&mut policy).unwrap();

        assert_eq!(policy.version, 2);
        assert_eq!(policy.default_access, McpAccessMode::None);
        assert_eq!(policy.rules.len(), 1);
        assert_eq!(policy.rules[0].access, McpAccessMode::ReadOnly);
        assert_eq!(policy.rules[0].prefix, "buggy-workspace");
    }

    #[test]
    fn nested_rule_longer_prefix_wins_over_shorter() {
        let storage = storage_with_policy(McpStoragePolicy {
            rules: vec![
                McpPathRule {
                    id: "short".to_string(),
                    prefix: "data".to_string(),
                    access: McpAccessMode::ReadOnly,
                    source: McpRuleSource::Manual,
                    confirmation_rules: None,
                },
                McpPathRule {
                    id: "long".to_string(),
                    prefix: "data/specific".to_string(),
                    access: McpAccessMode::ReadWrite,
                    source: McpRuleSource::Manual,
                    confirmation_rules: None,
                },
            ],
            default_access: McpAccessMode::None,
            ..Default::default()
        });

        let eval = evaluate_storage_policy(
            &storage,
            "data/specific/file.txt",
            McpOperation::Write,
            false,
            false,
        )
        .unwrap();
        assert_eq!(eval.matched_rule_id, Some("long".to_string()));

        let eval = evaluate_storage_policy(
            &storage,
            "data/general/file.txt",
            McpOperation::Write,
            false,
            false,
        )
        .unwrap_err();
        assert_eq!(eval.code, McpErrorCode::ERR_MCP_POLICY_DENIED);
    }

    #[test]
    fn multiple_workspaces_coexist() {
        let storage = storage_with_policy(McpStoragePolicy {
            rules: vec![
                McpPathRule {
                    id: "workspace:w-a".to_string(),
                    prefix: "team-a".to_string(),
                    access: McpAccessMode::ReadWrite,
                    source: McpRuleSource::Workspace {
                        workspace_id: "w-a".to_string(),
                    },
                    confirmation_rules: None,
                },
                McpPathRule {
                    id: "workspace:w-b".to_string(),
                    prefix: "team-b".to_string(),
                    access: McpAccessMode::ReadOnly,
                    source: McpRuleSource::Workspace {
                        workspace_id: "w-b".to_string(),
                    },
                    confirmation_rules: None,
                },
            ],
            default_access: McpAccessMode::None,
            ..Default::default()
        });

        let eval = evaluate_storage_policy(
            &storage,
            "team-a/src/main.rs",
            McpOperation::Read,
            false,
            false,
        )
        .unwrap();
        assert_eq!(eval.workspace_id, Some("w-a".to_string()));
        assert!(eval.matched_rule_id.is_some());

        let eval =
            evaluate_storage_policy(&storage, "team-b/doc.txt", McpOperation::Read, false, false)
                .unwrap();
        assert_eq!(eval.workspace_id, Some("w-b".to_string()));

        let eval = evaluate_storage_policy(
            &storage,
            "team-a/private/key.txt",
            McpOperation::Write,
            false,
            false,
        )
        .unwrap();
        assert_eq!(eval.workspace_id, Some("w-a".to_string()));

        let error = evaluate_storage_policy(
            &storage,
            "team-b/doc.txt",
            McpOperation::Write,
            false,
            false,
        )
        .unwrap_err();
        assert_eq!(error.code, McpErrorCode::ERR_MCP_POLICY_DENIED);
    }

    #[test]
    fn rule_level_confirmation_overrides_policy_level() {
        let storage = storage_with_policy(McpStoragePolicy {
            rules: vec![McpPathRule {
                id: "no-confirm".to_string(),
                prefix: "safe-zone".to_string(),
                access: McpAccessMode::ReadWrite,
                source: McpRuleSource::Manual,
                confirmation_rules: Some(McpConfirmationRules {
                    require_for_write: false,
                    require_for_delete: false,
                    ..Default::default()
                }),
            }],
            confirmation_rules: McpConfirmationRules {
                require_for_write: true,
                ..Default::default()
            },
            default_access: McpAccessMode::None,
            ..Default::default()
        });

        let eval = evaluate_storage_policy(
            &storage,
            "safe-zone/file.txt",
            McpOperation::Write,
            false,
            false,
        )
        .unwrap();
        assert_eq!(eval.decision, PolicyDecision::Allow);
    }

    #[test]
    fn versionless_policy_deserializes_as_v0() {
        let json = r#"{
            "default_access": "read_only",
            "rules": [],
            "denied_paths": [],
            "confirmation_rules": {
                "require_for_write": true,
                "require_for_overwrite": true,
                "require_for_delete": true,
                "require_for_version_delete": true,
                "require_for_presign": true,
                "require_for_cross_storage_copy": true
            }
        }"#;
        let policy: McpStoragePolicy = serde_json::from_str(json).unwrap();
        assert_eq!(
            policy.version, 0,
            "versionless policy must deserialize as v0"
        );
        assert_eq!(policy.default_access, McpAccessMode::ReadOnly);
        assert!(policy.rules.is_empty());
    }

    #[test]
    fn explicit_v0_policy_deserializes_correctly() {
        let json = r#"{
            "version": 0,
            "default_access": "read_only",
            "rules": [],
            "denied_paths": [],
            "confirmation_rules": {
                "require_for_write": true,
                "require_for_overwrite": true,
                "require_for_delete": true,
                "require_for_version_delete": true,
                "require_for_presign": true,
                "require_for_cross_storage_copy": true
            }
        }"#;
        let policy: McpStoragePolicy = serde_json::from_str(json).unwrap();
        assert_eq!(policy.version, 0);
    }

    #[test]
    fn normalize_storage_policy_rejects_duplicates() {
        let mut policy = McpStoragePolicy {
            rules: vec![
                McpPathRule {
                    id: "a".to_string(),
                    prefix: "projects".to_string(),
                    access: McpAccessMode::ReadOnly,
                    source: McpRuleSource::Manual,
                    confirmation_rules: None,
                },
                McpPathRule {
                    id: "b".to_string(),
                    prefix: "projects/".to_string(),
                    access: McpAccessMode::ReadWrite,
                    source: McpRuleSource::Manual,
                    confirmation_rules: None,
                },
            ],
            ..Default::default()
        };

        let error = normalize_storage_policy(&mut policy).unwrap_err();
        assert_eq!(error.code, McpErrorCode::ERR_INVALID_POLICY);
        assert!(error.message.contains("duplicate normalized rule prefix"));
    }

    #[test]
    fn duplicate_rule_ids_are_rejected_across_workspace_and_manual_rules() {
        let mut policy = McpStoragePolicy {
            rules: vec![
                McpPathRule {
                    id: "workspace:w-1".to_string(),
                    prefix: "workspace".to_string(),
                    access: McpAccessMode::ReadOnly,
                    source: McpRuleSource::Workspace {
                        workspace_id: "w-1".to_string(),
                    },
                    confirmation_rules: None,
                },
                McpPathRule {
                    id: "workspace:w-1".to_string(),
                    prefix: "manual".to_string(),
                    access: McpAccessMode::ReadOnly,
                    source: McpRuleSource::Manual,
                    confirmation_rules: None,
                },
            ],
            ..Default::default()
        };

        let error = normalize_storage_policy(&mut policy).unwrap_err();
        assert_eq!(error.code, McpErrorCode::ERR_INVALID_POLICY);
        assert_eq!(error.message, "duplicate policy rule ID 'workspace:w-1'");

        let storage = storage_with_policy(policy);
        let error =
            evaluate_storage_policy(&storage, "workspace/file", McpOperation::Read, false, false)
                .unwrap_err();
        assert_eq!(error.code, McpErrorCode::ERR_INVALID_POLICY);
        assert_eq!(
            error.message,
            "storage policy contains a duplicate persisted rule ID"
        );
    }

    #[test]
    fn unicode_control_chars_stripped_in_path_decoding() {
        // Control characters (except newline, tab, cr) should be stripped
        let path = "safe\u{0000}path\u{0001}/file";
        let decoded = decode_policy_path_controls(path);
        assert_eq!(decoded, "safepath/file");
    }

    #[test]
    fn unicode_characters_preserved_in_path_decoding() {
        // Unicode non-control characters should be preserved
        let path = "café/ résumé/ファイル";
        let decoded = decode_policy_path_controls(path);
        assert_eq!(decoded, path);
    }

    #[test]
    fn normalize_policy_path_handles_leading_slashes() {
        // Leading slashes are trimmed, so this succeeds
        let result = normalize_policy_path("//leading/slash").unwrap();
        assert_eq!(result, "leading/slash");
    }

    #[test]
    fn normalize_storage_policy_normalizes_denied_paths_and_checks_duplicates() {
        let mut policy = McpStoragePolicy {
            denied_paths: vec!["projects//secret".to_string(), "docs/./private".to_string()],
            rules: vec![McpPathRule {
                id: "r1".to_string(),
                prefix: "projects".to_string(),
                access: McpAccessMode::ReadOnly,
                source: McpRuleSource::Manual,
                confirmation_rules: None,
            }],
            ..Default::default()
        };

        normalize_storage_policy(&mut policy).unwrap();
        assert_eq!(policy.denied_paths, vec!["projects/secret", "docs/private"]);
        assert_eq!(policy.rules[0].prefix, "projects");
    }

    #[test]
    fn deserialize_policy_with_struct_default_then_migrate() {
        // Simulate old data that had version=1 via struct-level default
        let json = r#"{
            "version": 1,
            "default_access": "read_write",
            "rules": [],
            "denied_paths": [],
            "confirmation_rules": {
                "require_for_write": true,
                "require_for_overwrite": true,
                "require_for_delete": true,
                "require_for_version_delete": true,
                "require_for_presign": true,
                "require_for_cross_storage_copy": true
            },
            "allowed_paths": ["legacy-projects"]
        }"#;
        let mut policy: McpStoragePolicy = serde_json::from_str(json).unwrap();
        assert_eq!(policy.version, 1);
        migrate_legacy_policy(&mut policy).unwrap();
        assert_eq!(policy.version, MCP_POLICY_VERSION);
        assert_eq!(policy.rules.len(), 1);
        assert_eq!(policy.rules[0].prefix, "legacy-projects");
        assert!(policy.allowed_paths.is_empty());
    }
}
