use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;
use uuid::Uuid;

use crate::errors::{err_with_details, McpErrorCode, McpResult};
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
            allowed_prefixes: allowed_prefixes.unwrap_or_default(),
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

        if !session.allowed_storages.is_empty()
            && !session.allowed_storages.contains(&storage_name.to_string())
        {
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
                let normalized_path = normalize_session_path(path);
                let allowed = session
                    .allowed_prefixes
                    .iter()
                    .any(|prefix| session_path_matches_prefix(&normalized_path, prefix));
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

fn session_path_matches_prefix(path: &str, prefix: &str) -> bool {
    let prefix = normalize_session_path(prefix);
    if prefix.is_empty() {
        return true;
    }
    path == prefix
        || path
            .strip_prefix(&prefix)
            .is_some_and(|tail| tail.starts_with('/'))
}

fn normalize_session_path(path: &str) -> String {
    let mut segments = Vec::new();
    for segment in path.trim().replace('\\', "/").split('/') {
        match segment {
            "" | "." => {}
            ".." => {
                segments.pop();
            }
            value => segments.push(value.to_string()),
        }
    }
    segments.join("/")
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
}
