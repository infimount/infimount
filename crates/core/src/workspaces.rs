use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use crate::atomic_file;
use crate::models::{CoreError, Result};

pub const WORKSPACES_SCHEMA_VERSION: u32 = 1;
pub const WORKSPACES_FILE: &str = "workspaces.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceRecord {
    pub id: String,
    pub storage_id: String,
    pub name: String,
    pub root_path: String,
    pub template_id: String,
    pub created_at: String,
    pub updated_at: String,
    pub memory_files: Vec<String>,
    pub checkpoint_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WorkspacesFile {
    schema_version: u32,
    workspaces: Vec<WorkspaceRecord>,
}

pub struct WorkspaceRegistry {
    dir: PathBuf,
    cache: Mutex<HashMap<String, WorkspaceRecord>>,
}

impl WorkspaceRegistry {
    pub fn new(config_dir: &Path) -> Self {
        Self {
            dir: config_dir.to_path_buf(),
            cache: Mutex::new(HashMap::new()),
        }
    }

    fn path(&self) -> PathBuf {
        self.dir.join(WORKSPACES_FILE)
    }

    fn load_raw(&self) -> Result<WorkspacesFile> {
        let path = self.path();
        if !path.exists() {
            return Ok(WorkspacesFile {
                schema_version: WORKSPACES_SCHEMA_VERSION,
                workspaces: Vec::new(),
            });
        }
        let content = fs::read_to_string(&path)
            .map_err(|e| CoreError::Config(format!("failed to read workspace file: {e}")))?;
        let parsed: WorkspacesFile = serde_json::from_str(&content)
            .map_err(|e| CoreError::Config(format!("failed to parse workspace file: {e}")))?;
        Ok(parsed)
    }

    fn save_raw(&self, file: &WorkspacesFile) -> Result<()> {
        let payload = serde_json::to_vec_pretty(file)
            .map_err(|e| CoreError::Config(format!("failed to serialize workspaces: {e}")))?;
        atomic_file::atomic_write_file(&self.path(), &payload, atomic_file::FILE_MODE)
    }

    pub fn load_all(&self) -> Result<Vec<WorkspaceRecord>> {
        let file = self.load_raw()?;
        let mut cache = self.cache.lock().map_err(|e| {
            CoreError::Config(format!("workspace registry lock poisoned: {e}"))
        })?;
        cache.clear();
        for ws in &file.workspaces {
            cache.insert(ws.id.clone(), ws.clone());
        }
        Ok(file.workspaces)
    }

    pub fn save(&self, workspaces: &[WorkspaceRecord]) -> Result<()> {
        let file = WorkspacesFile {
            schema_version: WORKSPACES_SCHEMA_VERSION,
            workspaces: workspaces.to_vec(),
        };
        self.save_raw(&file)?;
        let mut cache = self.cache.lock().map_err(|e| {
            CoreError::Config(format!("workspace registry lock poisoned: {e}"))
        })?;
        cache.clear();
        for ws in workspaces {
            cache.insert(ws.id.clone(), ws.clone());
        }
        Ok(())
    }

    pub fn find_by_id(&self, id: &str) -> Result<Option<WorkspaceRecord>> {
        let cache = self.cache.lock().map_err(|e| {
            CoreError::Config(format!("workspace registry lock poisoned: {e}"))
        })?;
        Ok(cache.get(id).cloned())
    }

    pub fn create(&self, workspace: &WorkspaceRecord) -> Result<()> {
        let mut file = self.load_raw()?;
        if file.workspaces.iter().any(|w| w.id == workspace.id) {
            return Err(CoreError::Config(format!(
                "workspace '{}' already exists",
                workspace.id
            )));
        }
        file.workspaces.push(workspace.clone());
        self.save_raw(&file)?;
        let mut cache = self.cache.lock().map_err(|e| {
            CoreError::Config(format!("workspace registry lock poisoned: {e}"))
        })?;
        cache.insert(workspace.id.clone(), workspace.clone());
        Ok(())
    }

    pub fn update(&self, workspace: &WorkspaceRecord) -> Result<()> {
        let mut file = self.load_raw()?;
        let idx = file
            .workspaces
            .iter()
            .position(|w| w.id == workspace.id)
            .ok_or_else(|| {
                CoreError::Config(format!("workspace '{}' not found", workspace.id))
            })?;
        file.workspaces[idx] = workspace.clone();
        self.save_raw(&file)?;
        let mut cache = self.cache.lock().map_err(|e| {
            CoreError::Config(format!("workspace registry lock poisoned: {e}"))
        })?;
        cache.insert(workspace.id.clone(), workspace.clone());
        Ok(())
    }

    pub fn delete(&self, id: &str) -> Result<()> {
        let mut file = self.load_raw()?;
        let len_before = file.workspaces.len();
        file.workspaces.retain(|w| w.id != id);
        if file.workspaces.len() == len_before {
            return Err(CoreError::Config(format!(
                "workspace '{}' not found",
                id
            )));
        }
        self.save_raw(&file)?;
        let mut cache = self.cache.lock().map_err(|e| {
            CoreError::Config(format!("workspace registry lock poisoned: {e}"))
        })?;
        cache.remove(id);
        Ok(())
    }

    pub fn import_legacy(&self, legacy_workspaces: Vec<WorkspaceRecord>) -> Result<usize> {
        let mut file = self.load_raw()?;
        let mut imported = 0;
        for ws in legacy_workspaces {
            if !file.workspaces.iter().any(|w| w.id == ws.id) {
                file.workspaces.push(ws);
                imported += 1;
            }
        }
        if imported > 0 {
            self.save_raw(&file)?;
            let mut cache = self.cache.lock().map_err(|e| {
                CoreError::Config(format!("workspace registry lock poisoned: {e}"))
            })?;
            for ws in &file.workspaces {
                cache.insert(ws.id.clone(), ws.clone());
            }
        }
        Ok(imported)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_registry() -> (WorkspaceRegistry, tempfile::TempDir) {
        let dir = tempfile::tempdir().expect("temp dir");
        let registry = WorkspaceRegistry::new(dir.path());
        (registry, dir)
    }

    fn make_ws(id: &str) -> WorkspaceRecord {
        WorkspaceRecord {
            id: id.to_string(),
            storage_id: "local".to_string(),
            name: format!("Workspace {id}"),
            root_path: format!("/workspaces/{id}"),
            template_id: "coding".to_string(),
            created_at: "2025-01-01T00:00:00Z".to_string(),
            updated_at: "2025-01-01T00:00:00Z".to_string(),
            memory_files: vec!["memory/tasks.md".to_string()],
            checkpoint_ids: vec![],
        }
    }

    #[test]
    fn empty_registry_returns_empty_list() {
        let (registry, _dir) = test_registry();
        let list = registry.load_all().expect("load all");
        assert!(list.is_empty());
    }

    #[test]
    fn create_and_list() {
        let (registry, _dir) = test_registry();
        let ws = make_ws("ws-1");
        registry.create(&ws).expect("create");

        let list = registry.load_all().expect("load all");
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].id, "ws-1");
    }

    #[test]
    fn create_duplicate_id_fails() {
        let (registry, _dir) = test_registry();
        let ws = make_ws("ws-1");
        registry.create(&ws).expect("first create");
        let err = registry.create(&ws).unwrap_err();
        assert!(err.to_string().contains("already exists"));
    }

    #[test]
    fn update_modifies_record() {
        let (registry, _dir) = test_registry();
        let mut ws = make_ws("ws-1");
        registry.create(&ws).expect("create");

        ws.name = "Updated".to_string();
        registry.update(&ws).expect("update");

        let found = registry.find_by_id("ws-1").expect("find").unwrap();
        assert_eq!(found.name, "Updated");
    }

    #[test]
    fn update_nonexistent_fails() {
        let (registry, _dir) = test_registry();
        let ws = make_ws("missing");
        let err = registry.update(&ws).unwrap_err();
        assert!(err.to_string().contains("not found"));
    }

    #[test]
    fn delete_removes_record() {
        let (registry, _dir) = test_registry();
        let ws = make_ws("ws-1");
        registry.create(&ws).expect("create");
        registry.delete("ws-1").expect("delete");

        let list = registry.load_all().expect("load all");
        assert!(list.is_empty());
    }

    #[test]
    fn delete_nonexistent_fails() {
        let (registry, _dir) = test_registry();
        let err = registry.delete("missing").unwrap_err();
        assert!(err.to_string().contains("not found"));
    }

    #[test]
    fn import_legacy_skips_duplicates() {
        let (registry, _dir) = test_registry();
        let existing = make_ws("existing");
        registry.create(&existing).expect("create");

        let new = make_ws("new");
        let dup = make_ws("existing");

        let imported = registry
            .import_legacy(vec![new.clone(), dup])
            .expect("import");
        assert_eq!(imported, 1);

        let list = registry.load_all().expect("load all");
        assert_eq!(list.len(), 2);
    }

    #[test]
    fn persist_between_loads() {
        let dir = tempfile::tempdir().expect("temp dir");
        {
            let registry = WorkspaceRegistry::new(dir.path());
            registry.create(&make_ws("ws-1")).expect("create");
            registry.create(&make_ws("ws-2")).expect("create");
        }
        {
            let registry = WorkspaceRegistry::new(dir.path());
            let list = registry.load_all().expect("load all");
            assert_eq!(list.len(), 2);
        }
    }

    #[test]
    fn cache_is_updated_on_create() {
        let (registry, _dir) = test_registry();
        let ws = make_ws("ws-1");
        registry.create(&ws).expect("create");

        let cached = registry.find_by_id("ws-1").expect("find");
        assert!(cached.is_some());
    }
}
