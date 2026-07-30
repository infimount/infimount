use std::collections::HashMap;
use std::sync::Arc;

use opendal::Operator;

use crate::models::Result;
use crate::{registry, Source};

pub type StorageId = String;
pub type Revision = u64;

#[derive(Debug, Clone, Hash, PartialEq, Eq)]
pub struct CacheKey {
    pub storage_id: StorageId,
    pub revision: Revision,
}

#[derive(Debug, Clone)]
pub struct OperatorCache {
    operators: Arc<std::sync::Mutex<HashMap<CacheKey, Operator>>>,
}

impl OperatorCache {
    pub fn new() -> Self {
        Self {
            operators: Arc::new(std::sync::Mutex::new(HashMap::new())),
        }
    }

    pub fn get(&self, key: &CacheKey) -> Option<Operator> {
        let cache = self.operators.lock().ok()?;
        cache.get(key).cloned()
    }

    pub fn insert(&self, key: CacheKey, operator: Operator) {
        if let Ok(mut cache) = self.operators.lock() {
            cache.retain(|existing, _| existing.storage_id != key.storage_id);
            cache.insert(key, operator);
        }
    }

    pub fn invalidate(&self, storage_id: &str) {
        if let Ok(mut cache) = self.operators.lock() {
            cache.retain(|k, _| k.storage_id != storage_id);
        }
    }

    pub fn clear(&self) {
        if let Ok(mut cache) = self.operators.lock() {
            cache.clear();
        }
    }
}

impl Default for OperatorCache {
    fn default() -> Self {
        Self::new()
    }
}

pub fn get_or_create_operator(
    cache: &OperatorCache,
    source: &Source,
    revision: Revision,
) -> Result<Operator> {
    let cache_key = CacheKey {
        storage_id: source.id.clone(),
        revision,
    };

    if let Some(op) = cache.get(&cache_key) {
        return Ok(op);
    }

    let operator = registry::build_operator(source)?;
    cache.insert(cache_key, operator.clone());
    Ok(operator)
}

pub fn invalidate_source(cache: &OperatorCache, source_id: &str) {
    cache.invalidate(source_id);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{Source, SourceKind};

    fn make_source(id: &str, kind: SourceKind, root: &str) -> Source {
        Source {
            id: id.to_string(),
            name: id.to_string(),
            kind,
            root: root.to_string(),
            config: serde_json::Value::Null,
        }
    }

    #[test]
    fn test_cache_key_equality() {
        let a = CacheKey {
            storage_id: "s1".into(),
            revision: 1,
        };
        let b = CacheKey {
            storage_id: "s1".into(),
            revision: 1,
        };
        let c = CacheKey {
            storage_id: "s1".into(),
            revision: 2,
        };
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn newer_persisted_revision_evicts_the_previous_operator() {
        let cache = OperatorCache::new();
        let source = make_source("test", SourceKind::Local, "/tmp");
        get_or_create_operator(&cache, &source, 1).unwrap();
        get_or_create_operator(&cache, &source, 2).unwrap();
        let operators = cache.operators.lock().unwrap();
        assert_eq!(operators.len(), 1);
        assert!(!operators.contains_key(&CacheKey {
            storage_id: "test".into(),
            revision: 1,
        }));
        assert!(operators.contains_key(&CacheKey {
            storage_id: "test".into(),
            revision: 2,
        }));
    }

    #[test]
    fn test_cache_insert_get_invalidate() {
        let cache = OperatorCache::new();
        let source = make_source("test", SourceKind::Local, "/tmp");
        let op = get_or_create_operator(&cache, &source, 7).unwrap();
        assert!(op.info().full_capability().read);

        // Second call should return cached
        let op2 = get_or_create_operator(&cache, &source, 7).unwrap();
        assert!(op2.info().full_capability().read);

        // Invalidate
        invalidate_source(&cache, "test");
        let key = CacheKey {
            storage_id: "test".into(),
            revision: 7,
        };
        assert!(cache.get(&key).is_none());
    }
}
