use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;
use uuid::Uuid;

use crate::errors::{err_with_details, McpErrorCode, McpResult};
use crate::policy::normalize_policy_path;
use serde_json::json;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub id: String,
    pub allowed_storages: Vec<String>,
    pub allowed_prefixes: Vec<String>,
    pub read_only: bool,
    pub created_at: String,
    pub expires_at: String,
}

#[derive(Debug, Clone)]
pub struct SessionWithMeta {
    pub session: Session,
    pub expires_instant: Instant,
}

#[derive(Debug, Clone)]
pub struct SessionManager {
    sessions: Arc<RwLock<HashMap<String, SessionWithMeta>>>,
}

impl SessionManager {
    pub fn new() -> Self {
        Self {
            sessions: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub async fn create_session(
        &self,
        allowed_storages: Vec<String>,
        allowed_prefixes: Option<Vec<String>>,
        read_only: Option<bool>,
        ttl_seconds: Option<u64>,
    ) -> McpResult<Session> {
        if allowed_storages.is_empty() {
            return Err(err_with_details(
                McpErrorCode::ERR_INVALID_PATH,
                "allowed_storages must not be empty",
                json!({}),
            ));
        }

        // Normalize all allowed prefixes using centralized policy path normalization
        let normalized_prefixes = match allowed_prefixes {
            Some(prefixes) => {
                let mut normalized = Vec::with_capacity(prefixes.len());
                for prefix in &prefixes {
                    normalized.push(normalize_policy_path(prefix)?);
                }
                normalized
            }
            None => Vec::new(),
        };

        let ttl = ttl_seconds.unwrap_or(3600);
        if ttl == 0 || ttl > 86400 {
            return Err(err_with_details(
                McpErrorCode::ERR_INVALID_PATH,
                "ttl_seconds must be between 1 and 86400",
                json!({ "ttl_seconds": ttl }),
            ));
        }

        let now = Instant::now();
        let expires_instant = now + Duration::from_secs(ttl);

        let session = Session {
            id: Uuid::new_v4().to_string(),
            allowed_storages,
            allowed_prefixes: normalized_prefixes,
            read_only: read_only.unwrap_or(false),
            created_at: chrono::Utc::now().to_rfc3339(),
            expires_at: chrono::Utc::now()
                .checked_add_signed(chrono::Duration::seconds(ttl as i64))
                .unwrap_or_else(chrono::Utc::now)
                .to_rfc3339(),
        };

        let mut sessions = self.sessions.write().await;
        sessions.insert(
            session.id.clone(),
            SessionWithMeta {
                session: session.clone(),
                expires_instant,
            },
        );

        Ok(session)
    }

    pub async fn end_session(&self, session_id: &str) -> McpResult<bool> {
        let mut sessions = self.sessions.write().await;
        if sessions.remove(session_id).is_some() {
            Ok(true)
        } else {
            Err(err_with_details(
                McpErrorCode::ERR_SESSION_NOT_FOUND,
                "session not found",
                json!({ "session_id": session_id }),
            ))
        }
    }

    pub async fn list_active(&self) -> Vec<Session> {
        self.cleanup_expired().await;
        let mut sessions = self
            .sessions
            .read()
            .await
            .values()
            .map(|item| item.session.clone())
            .collect::<Vec<_>>();
        sessions.sort_by(|a, b| a.created_at.cmp(&b.created_at));
        sessions
    }

    pub async fn get_session(&self, session_id: &str) -> McpResult<Session> {
        let sessions = self.sessions.read().await;

        if let Some(session_meta) = sessions.get(session_id) {
            if session_meta.expires_instant > Instant::now() {
                Ok(session_meta.session.clone())
            } else {
                drop(sessions);
                self.end_session(session_id).await?;
                Err(err_with_details(
                    McpErrorCode::ERR_SESSION_NOT_FOUND,
                    "session has expired",
                    json!({ "session_id": session_id }),
                ))
            }
        } else {
            Err(err_with_details(
                McpErrorCode::ERR_SESSION_NOT_FOUND,
                "session not found",
                json!({ "session_id": session_id }),
            ))
        }
    }

    pub async fn validate_access(
        &self,
        session_id: &str,
        storage_name: &str,
        backend_path: Option<&str>,
    ) -> McpResult<bool> {
        let session = self.get_session(session_id).await?;

        if !session.allowed_storages.contains(&storage_name.to_string()) {
            return Err(err_with_details(
                McpErrorCode::ERR_SESSION_FORBIDDEN,
                "storage not allowed in session",
                json!({
                    "session_id": session_id,
                    "storage": storage_name
                }),
            ));
        }

        if let Some(path) = backend_path {
            if !session.allowed_prefixes.is_empty() {
                let normalized_path = normalize_policy_path(path)?;
                let allowed = session
                    .allowed_prefixes
                    .iter()
                    .any(|prefix| path_matches_prefix(&normalized_path, prefix));
                if !allowed {
                    return Err(err_with_details(
                        McpErrorCode::ERR_SESSION_FORBIDDEN,
                        "path not allowed in session",
                        json!({
                            "session_id": session_id,
                            "path": path
                        }),
                    ));
                }
            }
        }

        Ok(!session.read_only)
    }

    pub async fn cleanup_expired(&self) {
        let now = Instant::now();
        let mut sessions = self.sessions.write().await;
        sessions.retain(|_, meta| meta.expires_instant > now);
    }

    pub async fn clear(&self) {
        self.sessions.write().await.clear();
    }
}

fn path_matches_prefix(path: &str, prefix: &str) -> bool {
    if prefix.is_empty() {
        return true;
    }
    path == prefix
        || path
            .strip_prefix(prefix)
            .is_some_and(|tail| tail.starts_with('/'))
}

impl Default for SessionManager {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionCreateInput {
    pub allowed_storages: Vec<String>,
    #[serde(default)]
    pub allowed_prefixes: Option<Vec<String>>,
    #[serde(default)]
    pub read_only: Option<bool>,
    #[serde(default)]
    pub ttl_seconds: Option<u64>,
}

#[derive(Debug, Serialize)]
pub struct SessionCreateOutput {
    pub session_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionEndInput {
    pub session_id: String,
}

#[derive(Debug, Serialize)]
pub struct SessionEndOutput {
    pub session_id: String,
    pub ended: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn rejects_empty_allowed_storages() {
        let manager = SessionManager::new();
        let error = manager
            .create_session(vec![], None, Some(true), Some(60))
            .await
            .unwrap_err();
        assert_eq!(error.code, McpErrorCode::ERR_INVALID_PATH);
        assert!(error.message.contains("must not be empty"));
    }

    #[tokio::test]
    async fn list_active_returns_non_expired_sessions_only() {
        let manager = SessionManager::new();
        let session = manager
            .create_session(vec!["Local".to_string()], None, Some(true), Some(60))
            .await
            .unwrap();

        let active = manager.list_active().await;

        assert_eq!(active.len(), 1);
        assert_eq!(active[0].id, session.id);
        assert_eq!(active[0].allowed_storages, vec!["Local".to_string()]);
        assert!(active[0].read_only);
    }

    #[tokio::test]
    async fn session_prefixes_are_normalized_and_segment_aware() {
        let manager = SessionManager::new();
        let session = manager
            .create_session(
                vec!["Local".to_string()],
                Some(vec!["docs".to_string()]),
                None,
                Some(60),
            )
            .await
            .unwrap();

        assert!(manager
            .validate_access(&session.id, "Local", Some("docs/file.txt"))
            .await
            .unwrap());
        assert!(manager
            .validate_access(&session.id, "Local", Some("./docs\\nested.txt"))
            .await
            .unwrap());
        assert!(manager
            .validate_access(&session.id, "Local", Some("docs2/file.txt"))
            .await
            .is_err());
        assert!(manager
            .validate_access(&session.id, "Local", Some("docs/../private.txt"))
            .await
            .is_err());
    }

    #[tokio::test]
    async fn session_denies_encoded_escape_attempts() {
        let manager = SessionManager::new();
        let session = manager
            .create_session(
                vec!["Local".to_string()],
                Some(vec!["safe".to_string()]),
                None,
                Some(60),
            )
            .await
            .unwrap();

        // %2e%2e is decoded to ".." which escapes from "safe" prefix
        assert!(manager
            .validate_access(&session.id, "Local", Some("safe/%2e%2e/outside.txt"))
            .await
            .is_err());
        // %2f is decoded to "/" which creates a path component boundary -> normalized to "safe/outside.txt"
        assert!(manager
            .validate_access(&session.id, "Local", Some("safe%2foutside.txt"))
            .await
            .is_ok());
        // Backslash decoded to "/"
        assert!(manager
            .validate_access(&session.id, "Local", Some("safe\\outside.txt"))
            .await
            .is_ok());
        // Double dot with encoded variants
        assert!(manager
            .validate_access(&session.id, "Local", Some("safe/./outside.txt"))
            .await
            .is_ok());
    }

    #[tokio::test]
    async fn session_prefix_normalization_handles_encoded_slashes_dots_and_backslashes() {
        let manager = SessionManager::new();
        // Prefix with trailing slash should be normalized
        let session = manager
            .create_session(
                vec!["Local".to_string()],
                Some(vec!["projects/".to_string()]),
                None,
                Some(60),
            )
            .await
            .unwrap();

        assert_eq!(session.allowed_prefixes, vec!["projects"]);
    }

    #[tokio::test]
    async fn session_validates_multiple_storages() {
        let manager = SessionManager::new();
        let session = manager
            .create_session(
                vec!["Local".to_string(), "Remote".to_string()],
                None,
                None,
                Some(60),
            )
            .await
            .unwrap();

        assert!(manager
            .validate_access(&session.id, "Local", None)
            .await
            .is_ok());
        assert!(manager
            .validate_access(&session.id, "Remote", None)
            .await
            .is_ok());
        assert!(manager
            .validate_access(&session.id, "Other", None)
            .await
            .is_err());
    }

    #[tokio::test]
    async fn ttl_validation_rejects_zero_and_excessive_values() {
        let manager = SessionManager::new();
        assert!(manager
            .create_session(vec!["Local".to_string()], None, None, Some(0))
            .await
            .is_err());
        assert!(manager
            .create_session(vec!["Local".to_string()], None, None, Some(86401))
            .await
            .is_err());
        assert!(manager
            .create_session(vec!["Local".to_string()], None, None, Some(3600))
            .await
            .is_ok());
    }

    #[tokio::test]
    async fn end_session_removes_active_session() {
        let manager = SessionManager::new();
        let session = manager
            .create_session(vec!["Local".to_string()], None, None, Some(60))
            .await
            .unwrap();

        assert!(manager.end_session(&session.id).await.unwrap());
        assert!(manager.end_session(&session.id).await.is_err());
        assert!(manager.get_session(&session.id).await.is_err());
    }

    #[tokio::test]
    async fn session_cleanup_removes_expired() {
        let manager = SessionManager::new();
        let session = manager
            .create_session(vec!["Local".to_string()], None, None, Some(1))
            .await
            .unwrap();

        assert!(manager.get_session(&session.id).await.is_ok());
        tokio::time::sleep(Duration::from_millis(1100)).await;
        assert!(manager.get_session(&session.id).await.is_err());
        let active = manager.list_active().await;
        assert!(active.is_empty());
    }

    #[tokio::test]
    async fn clear_removes_all_sessions() {
        let manager = SessionManager::new();
        manager
            .create_session(vec!["Local".to_string()], None, None, Some(60))
            .await
            .unwrap();
        manager
            .create_session(vec!["Remote".to_string()], None, None, Some(60))
            .await
            .unwrap();

        manager.clear().await;
        assert!(manager.list_active().await.is_empty());
    }
}
