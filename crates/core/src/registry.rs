use std::collections::HashMap;

use opendal::services::Fs;
use opendal::Operator;
use tokio::sync::RwLock;

use crate::config;
use crate::models::{CoreError, Result, Source, SourceKind};

/// Registry that maps source IDs to OpenDAL operators.
///
/// Operators are built lazily from `Source` configuration and cached.
pub struct OperatorRegistry {
    sources: RwLock<HashMap<String, Source>>,
    operators: RwLock<HashMap<String, Operator>>,
}

impl OperatorRegistry {
    /// Create a new registry from a list of configured sources.
    pub fn new(sources: Vec<Source>) -> Self {
        let mut map = HashMap::new();
        for src in sources {
            map.insert(src.id.clone(), src);
        }

        Self {
            sources: RwLock::new(map),
            operators: RwLock::new(HashMap::new()),
        }
    }

    /// Return all known sources.
    pub async fn list_sources(&self) -> Vec<Source> {
        self.sources
            .read()
            .await
            .values()
            .cloned()
            .collect::<Vec<_>>()
    }

    /// Replace all sources with the provided list, clearing any
    /// cached operators and persisting the new configuration.
    pub async fn replace_sources(&self, sources: Vec<Source>) -> Result<()> {
        {
            let mut srcs = self.sources.write().await;
            srcs.clear();
            for s in sources {
                srcs.insert(s.id.clone(), s);
            }
        }

        // Clear all cached operators – they will be rebuilt lazily.
        {
            let mut ops = self.operators.write().await;
            ops.clear();
        }

        let all_sources = self.list_sources().await;
        config::save_sources(&all_sources)?;
        Ok(())
    }

    /// Add a new source and persist configuration.
    pub async fn add_source(&self, source: Source) -> Result<()> {
        {
            let mut sources = self.sources.write().await;
            sources.insert(source.id.clone(), source.clone());
        }

        // Clear any existing operator cached for this source (if any).
        {
            let mut ops = self.operators.write().await;
            ops.remove(&source.id);
        }

        // Persist the updated list.
        let all_sources = self.list_sources().await;
        config::save_sources(&all_sources)?;
        Ok(())
    }

    /// Remove a source by id and persist configuration.
    pub async fn remove_source(&self, source_id: &str) -> Result<()> {
        {
            let mut sources = self.sources.write().await;
            sources.remove(source_id);
        }

        // Remove cached operator reference.
        {
            let mut ops = self.operators.write().await;
            ops.remove(source_id);
        }

        let all_sources = self.list_sources().await;
        config::save_sources(&all_sources)?;
        Ok(())
    }

    /// Update an existing source (or add if missing) and persist.
    pub async fn update_source(&self, source: Source) -> Result<()> {
        {
            let mut sources = self.sources.write().await;
            sources.insert(source.id.clone(), source.clone());
        }

        // When updated, clear the operator cache for that source.
        {
            let mut ops = self.operators.write().await;
            ops.remove(&source.id);
        }

        let all_sources = self.list_sources().await;
        config::save_sources(&all_sources)?;
        Ok(())
    }

    /// Get (or lazily build) an operator for the given source ID.
    pub async fn get_operator(&self, source_id: &str) -> Result<Operator> {
        // Fast path: already built.
        if let Some(op) = self.operators.read().await.get(source_id) {
            return Ok(op.clone());
        }

        // Load source configuration.
        let source = {
            let sources = self.sources.read().await;
            sources
                .get(source_id)
                .cloned()
                .ok_or_else(|| CoreError::SourceNotFound(source_id.to_string()))?
        };

        // Build a new operator for this source.
        let op = build_operator(&source)?;

        // Cache and return.
        let mut ops = self.operators.write().await;
        ops.insert(source_id.to_string(), op.clone());
        Ok(op)
    }
}

fn build_operator(source: &Source) -> Result<Operator> {
    match source.kind {
        SourceKind::Local => build_local_operator(&source.root),
        // Other backends will be added incrementally.
        _ => Err(CoreError::UnsupportedSourceKind(source.kind.clone())),
    }
}

fn build_local_operator(root: &str) -> Result<Operator> {
    let builder = Fs::default().root(root);
    let op = Operator::new(builder)
        .map_err(CoreError::Storage)?
        .finish();
    Ok(op)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::Source;
    use std::env;
    use std::fs;
    use std::path::PathBuf;

    fn test_config_path() -> PathBuf {
        let mut p = env::temp_dir();
        p.push("openhsb_test_config.json");
        p
    }

    fn reset_config_file(path: &PathBuf) {
        let _ = fs::remove_file(path);
    }

    #[tokio::test]
    async fn add_remove_source_persists() {
        let cfg = test_config_path();
        reset_config_file(&cfg);
        env::set_var("OPENHSB_CONFIG", &cfg);

        let registry = OperatorRegistry::new(vec![]);

        let s = Source {
            id: "test1".to_string(),
            name: "Test1".to_string(),
            kind: crate::models::SourceKind::Local,
            root: "/tmp".to_string(),
        };

        registry.add_source(s.clone()).await.unwrap();
        let sources = registry.list_sources().await;
        assert_eq!(sources.len(), 1);
        assert_eq!(sources[0].id, "test1");

        registry.remove_source("test1").await.unwrap();
        let sources = registry.list_sources().await;
        assert_eq!(sources.len(), 0);

        // cleanup
        let _ = fs::remove_file(cfg);
    }
}
