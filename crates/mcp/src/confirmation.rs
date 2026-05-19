use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::sync::RwLock;
use uuid::Uuid;

use crate::errors::{err_with_details, McpErrorCode, McpResult};
use crate::policy::{McpOperation, McpRiskType};

const DEFAULT_CONFIRMATION_TTL_SECONDS: u64 = 300;
const DEFAULT_CONFIRMATION_QUEUE_LIMIT: usize = 100;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfirmationDecision {
    Pending,
    Approved,
    Denied,
    Expired,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingConfirmation {
    pub operation_id: String,
    pub tool_name: String,
    pub operation: McpOperation,
    pub risk_type: McpRiskType,
    pub storage_id: String,
    pub storage_name: String,
    pub path: String,
    pub summary: String,
    pub request_fingerprint: String,
    pub created_at: String,
    pub expires_at: String,
}

#[derive(Debug, Clone)]
pub struct ConfirmationRequest {
    pub tool_name: String,
    pub operation: McpOperation,
    pub risk_type: McpRiskType,
    pub storage_id: String,
    pub storage_name: String,
    pub path: String,
    pub summary: String,
    pub request_fingerprint: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfirmationRequiredResponse {
    pub status: String,
    pub operation_id: String,
    pub operation_summary: String,
    pub storage_id: String,
    pub storage_name: String,
    pub path: String,
    pub tool_name: String,
    pub risk_type: McpRiskType,
    pub expires_at: String,
}

impl From<PendingConfirmation> for ConfirmationRequiredResponse {
    fn from(value: PendingConfirmation) -> Self {
        Self {
            status: "requires_confirmation".to_string(),
            operation_id: value.operation_id,
            operation_summary: value.summary,
            storage_id: value.storage_id,
            storage_name: value.storage_name,
            path: value.path,
            tool_name: value.tool_name,
            risk_type: value.risk_type,
            expires_at: value.expires_at,
        }
    }
}

#[derive(Debug, Clone)]
struct PendingConfirmationWithMeta {
    pending: PendingConfirmation,
    expires_instant: Instant,
}

#[derive(Debug, Clone)]
pub struct ConfirmationManager {
    pending: Arc<RwLock<HashMap<String, PendingConfirmationWithMeta>>>,
    approved: Arc<RwLock<HashMap<String, PendingConfirmationWithMeta>>>,
    ttl: Duration,
    limit: usize,
}

impl ConfirmationManager {
    pub fn new() -> Self {
        Self::with_ttl(Duration::from_secs(DEFAULT_CONFIRMATION_TTL_SECONDS))
    }

    pub fn with_ttl(ttl: Duration) -> Self {
        Self::with_ttl_and_limit(ttl, DEFAULT_CONFIRMATION_QUEUE_LIMIT)
    }

    pub fn with_ttl_and_limit(ttl: Duration, limit: usize) -> Self {
        Self {
            pending: Arc::new(RwLock::new(HashMap::new())),
            approved: Arc::new(RwLock::new(HashMap::new())),
            ttl,
            limit,
        }
    }

    pub async fn require_confirmation(&self, request: ConfirmationRequest) -> PendingConfirmation {
        self.cleanup_expired().await;
        let operation_id = Uuid::new_v4().to_string();
        let now = Utc::now();
        let expires_at = now
            .checked_add_signed(chrono::Duration::from_std(self.ttl).unwrap_or_default())
            .unwrap_or(now)
            .to_rfc3339();
        let pending = PendingConfirmation {
            operation_id: operation_id.clone(),
            tool_name: request.tool_name,
            operation: request.operation,
            risk_type: request.risk_type,
            storage_id: request.storage_id,
            storage_name: request.storage_name,
            path: request.path,
            summary: request.summary,
            request_fingerprint: request.request_fingerprint,
            created_at: now.to_rfc3339(),
            expires_at,
        };

        let mut guard = self.pending.write().await;
        guard.insert(
            operation_id,
            PendingConfirmationWithMeta {
                pending: pending.clone(),
                expires_instant: Instant::now() + self.ttl,
            },
        );
        enforce_pending_limit(&mut guard, self.limit);
        pending
    }

    pub async fn list_pending(&self) -> Vec<PendingConfirmation> {
        self.cleanup_expired().await;
        let mut items = self
            .pending
            .read()
            .await
            .values()
            .map(|item| item.pending.clone())
            .collect::<Vec<_>>();
        items.sort_by(|a, b| a.created_at.cmp(&b.created_at));
        items
    }

    pub async fn approve(&self, operation_id: &str) -> McpResult<PendingConfirmation> {
        let item = self
            .remove_pending(operation_id, ConfirmationDecision::Approved)
            .await?;
        let pending = item.pending.clone();
        self.approved
            .write()
            .await
            .insert(operation_id.to_string(), item);
        Ok(pending)
    }

    pub async fn deny(&self, operation_id: &str) -> McpResult<PendingConfirmation> {
        self.remove_pending(operation_id, ConfirmationDecision::Denied)
            .await
            .map(|item| item.pending)
    }

    pub async fn consume_approved(
        &self,
        operation_id: &str,
        request_fingerprint: &str,
    ) -> McpResult<PendingConfirmation> {
        let mut guard = self.approved.write().await;
        let Some(item) = guard.remove(operation_id) else {
            return Err(err_with_details(
                McpErrorCode::ERR_CONFIRMATION_REQUIRED,
                "confirmation operation has not been approved",
                json!({ "operation_id": operation_id }),
            ));
        };

        if item.expires_instant <= Instant::now() {
            return Err(err_with_details(
                McpErrorCode::ERR_CONFIRMATION_REQUIRED,
                "approved confirmation operation has expired",
                json!({
                    "operation_id": operation_id,
                    "decision": ConfirmationDecision::Expired
                }),
            ));
        }

        if item.pending.request_fingerprint != request_fingerprint {
            return Err(err_with_details(
                McpErrorCode::ERR_CONFIRMATION_REQUIRED,
                "confirmation request does not match approved operation",
                json!({ "operation_id": operation_id }),
            ));
        }

        Ok(item.pending)
    }

    async fn remove_pending(
        &self,
        operation_id: &str,
        decision: ConfirmationDecision,
    ) -> McpResult<PendingConfirmationWithMeta> {
        let mut guard = self.pending.write().await;
        let Some(item) = guard.remove(operation_id) else {
            return Err(err_with_details(
                McpErrorCode::ERR_CONFIRMATION_REQUIRED,
                "confirmation operation is not pending",
                json!({ "operation_id": operation_id }),
            ));
        };

        if item.expires_instant <= Instant::now() {
            return Err(err_with_details(
                McpErrorCode::ERR_CONFIRMATION_REQUIRED,
                "confirmation operation has expired",
                json!({
                    "operation_id": operation_id,
                    "decision": ConfirmationDecision::Expired
                }),
            ));
        }

        let _ = decision;
        Ok(item)
    }

    pub async fn cleanup_expired(&self) {
        let now = Instant::now();
        let mut guard = self.pending.write().await;
        guard.retain(|_, item| item.expires_instant > now);
        drop(guard);
        let mut approved = self.approved.write().await;
        approved.retain(|_, item| item.expires_instant > now);
    }
}

fn enforce_pending_limit(pending: &mut HashMap<String, PendingConfirmationWithMeta>, limit: usize) {
    while pending.len() > limit {
        let Some(oldest_id) = pending
            .iter()
            .min_by(|(_, a), (_, b)| a.pending.created_at.cmp(&b.pending.created_at))
            .map(|(operation_id, _)| operation_id.clone())
        else {
            break;
        };
        pending.remove(&oldest_id);
    }
}

impl Default for ConfirmationManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    #[tokio::test]
    async fn confirmation_lifecycle_approves_once_and_prevents_replay() {
        let manager = ConfirmationManager::new();
        let pending = manager
            .require_confirmation(ConfirmationRequest {
                tool_name: "delete_path".to_string(),
                operation: McpOperation::Delete,
                risk_type: McpRiskType::Delete,
                storage_id: "storage-id".to_string(),
                storage_name: "Local".to_string(),
                path: "/Local/a.txt".to_string(),
                summary: "Delete /Local/a.txt".to_string(),
                request_fingerprint: "delete:v1".to_string(),
            })
            .await;

        assert_eq!(manager.list_pending().await.len(), 1);
        let approved = manager.approve(&pending.operation_id).await.unwrap();
        assert_eq!(approved.operation_id, pending.operation_id);
        assert!(manager.approve(&pending.operation_id).await.is_err());
        assert!(manager
            .consume_approved(&pending.operation_id, "delete:v1")
            .await
            .is_ok());
        assert!(manager
            .consume_approved(&pending.operation_id, "delete:v1")
            .await
            .is_err());
        assert!(manager.list_pending().await.is_empty());
    }

    #[tokio::test]
    async fn confirmation_lifecycle_deny_removes_pending_operation() {
        let manager = ConfirmationManager::new();
        let pending = manager
            .require_confirmation(ConfirmationRequest {
                tool_name: "write_file".to_string(),
                operation: McpOperation::Write,
                risk_type: McpRiskType::Write,
                storage_id: "storage-id".to_string(),
                storage_name: "Local".to_string(),
                path: "/Local/a.txt".to_string(),
                summary: "Write /Local/a.txt".to_string(),
                request_fingerprint: "write:v1".to_string(),
            })
            .await;

        let denied = manager.deny(&pending.operation_id).await.unwrap();
        assert_eq!(denied.operation_id, pending.operation_id);
        assert!(manager.list_pending().await.is_empty());
    }

    #[tokio::test]
    async fn confirmation_lifecycle_expires_pending_operations() {
        let manager = ConfirmationManager::with_ttl(Duration::from_millis(5));
        let pending = manager
            .require_confirmation(ConfirmationRequest {
                tool_name: "delete_path".to_string(),
                operation: McpOperation::Delete,
                risk_type: McpRiskType::Delete,
                storage_id: "storage-id".to_string(),
                storage_name: "Local".to_string(),
                path: "/Local/a.txt".to_string(),
                summary: "Delete /Local/a.txt".to_string(),
                request_fingerprint: "delete:v1".to_string(),
            })
            .await;

        tokio::time::sleep(Duration::from_millis(10)).await;
        assert!(manager.approve(&pending.operation_id).await.is_err());
        assert!(manager.list_pending().await.is_empty());
    }

    #[tokio::test]
    async fn confirmation_lifecycle_rejects_tampered_request() {
        let manager = ConfirmationManager::new();
        let pending = manager
            .require_confirmation(ConfirmationRequest {
                tool_name: "delete_path".to_string(),
                operation: McpOperation::Delete,
                risk_type: McpRiskType::Delete,
                storage_id: "storage-id".to_string(),
                storage_name: "Local".to_string(),
                path: "/Local/a.txt".to_string(),
                summary: "Delete /Local/a.txt".to_string(),
                request_fingerprint: "delete:a".to_string(),
            })
            .await;

        manager.approve(&pending.operation_id).await.unwrap();

        assert!(manager
            .consume_approved(&pending.operation_id, "delete:b")
            .await
            .is_err());
        assert!(manager
            .consume_approved(&pending.operation_id, "delete:a")
            .await
            .is_err());
    }

    #[tokio::test]
    async fn confirmation_state_is_in_memory_and_cleared_on_restart() {
        let manager = ConfirmationManager::new();
        let pending = manager
            .require_confirmation(ConfirmationRequest {
                tool_name: "delete_path".to_string(),
                operation: McpOperation::Delete,
                risk_type: McpRiskType::Delete,
                storage_id: "storage-id".to_string(),
                storage_name: "Local".to_string(),
                path: "/Local/a.txt".to_string(),
                summary: "Delete /Local/a.txt".to_string(),
                request_fingerprint: "delete:v1".to_string(),
            })
            .await;
        let restarted_manager = ConfirmationManager::new();

        assert!(restarted_manager
            .approve(&pending.operation_id)
            .await
            .is_err());
        assert!(restarted_manager
            .consume_approved(&pending.operation_id, "delete:v1")
            .await
            .is_err());
    }

    #[tokio::test]
    async fn confirmation_queue_is_bounded_and_evicts_oldest_pending_items() {
        let manager = ConfirmationManager::with_ttl_and_limit(Duration::from_secs(300), 2);
        let first = manager
            .require_confirmation(ConfirmationRequest {
                tool_name: "delete_path".to_string(),
                operation: McpOperation::Delete,
                risk_type: McpRiskType::Delete,
                storage_id: "storage-id".to_string(),
                storage_name: "Local".to_string(),
                path: "/Local/one.txt".to_string(),
                summary: "Delete /Local/one.txt".to_string(),
                request_fingerprint: "delete:one".to_string(),
            })
            .await;
        let second = manager
            .require_confirmation(ConfirmationRequest {
                tool_name: "delete_path".to_string(),
                operation: McpOperation::Delete,
                risk_type: McpRiskType::Delete,
                storage_id: "storage-id".to_string(),
                storage_name: "Local".to_string(),
                path: "/Local/two.txt".to_string(),
                summary: "Delete /Local/two.txt".to_string(),
                request_fingerprint: "delete:two".to_string(),
            })
            .await;
        let third = manager
            .require_confirmation(ConfirmationRequest {
                tool_name: "delete_path".to_string(),
                operation: McpOperation::Delete,
                risk_type: McpRiskType::Delete,
                storage_id: "storage-id".to_string(),
                storage_name: "Local".to_string(),
                path: "/Local/three.txt".to_string(),
                summary: "Delete /Local/three.txt".to_string(),
                request_fingerprint: "delete:three".to_string(),
            })
            .await;

        let pending = manager.list_pending().await;
        assert_eq!(pending.len(), 2);
        assert!(!pending
            .iter()
            .any(|item| item.operation_id == first.operation_id));
        assert!(pending
            .iter()
            .any(|item| item.operation_id == second.operation_id));
        assert!(pending
            .iter()
            .any(|item| item.operation_id == third.operation_id));
    }
}
