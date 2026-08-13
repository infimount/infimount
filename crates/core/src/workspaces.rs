use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::path::{Path, PathBuf};

use fs2::FileExt;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::atomic_file;
use crate::models::{CoreError, Result};

pub const WORKSPACES_SCHEMA_VERSION: u32 = 1;
pub const WORKSPACE_RECORD_SCHEMA_VERSION: u32 = 1;
pub const WORKSPACES_FILE: &str = "workspaces.json";
pub const MAX_CHECKPOINT_IDS: usize = 200;
const WORKSPACE_MUTATION_LOCK_FILE: &str = "workspace-mutations.lock";

/// Holds the cross-process lock for a complete workspace command transaction.
///
/// Registry methods have their own shorter file lock. This separate lock lets the
/// desktop command layer serialize root validation, remote manifest writes, registry
/// changes, and MCP policy changes as one transaction.
pub struct WorkspaceMutationGuard {
    _file: File,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceRecord {
    pub id: String,
    #[serde(default = "default_schema_version")]
    pub schema_version: u32,
    pub storage_id: String,
    pub name: String,
    pub root_path: String,
    pub template_id: String,
    #[serde(default = "default_access_profile")]
    pub access_profile: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub policy_rule_id: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub memory_files: Vec<String>,
    pub checkpoint_ids: Vec<String>,
}

fn default_schema_version() -> u32 {
    WORKSPACE_RECORD_SCHEMA_VERSION
}

fn default_access_profile() -> String {
    "read_only".to_string()
}

pub fn generate_workspace_id() -> String {
    Uuid::new_v4().to_string()
}

pub fn known_template_plans() -> Vec<&'static str> {
    vec![
        "coding",
        "writing",
        "research",
        "data-analysis",
        "admin",
        "custom",
    ]
}

pub fn memory_files_for(template_id: &str) -> Vec<String> {
    match template_id {
        "coding" => vec![
            "memory/tasks.md".to_string(),
            "memory/decisions.md".to_string(),
            "memory/handoff.md".to_string(),
        ],
        "research" => vec![
            "memory/questions.md".to_string(),
            "memory/sources.md".to_string(),
            "memory/summary.md".to_string(),
        ],
        "writing" => vec![
            "memory/outline.md".to_string(),
            "memory/notes.md".to_string(),
        ],
        "data-analysis" => vec![
            "memory/datasets.md".to_string(),
            "memory/observations.md".to_string(),
            "memory/runbook.md".to_string(),
        ],
        "admin" => vec![],
        _ => vec![],
    }
}

#[derive(Debug, Clone)]
pub struct TemplateFile {
    pub path: String,
    pub content: String,
}

pub fn validate_workspace_metadata(workspace: &WorkspaceRecord) -> Result<()> {
    let expected_memory_files = memory_files_for(&workspace.template_id);
    if workspace.memory_files != expected_memory_files {
        return Err(CoreError::Config(
            "workspace memory files must match its trusted template plan".to_string(),
        ));
    }
    if workspace.checkpoint_ids.len() > MAX_CHECKPOINT_IDS {
        return Err(CoreError::Config(format!(
            "workspace may contain at most {MAX_CHECKPOINT_IDS} checkpoint IDs"
        )));
    }
    let mut seen = std::collections::HashSet::new();
    for id in &workspace.checkpoint_ids {
        if id.len() > 128
            || !id.starts_with("checkpoint-")
            || !id.chars().all(|ch| ch.is_ascii_alphanumeric() || ch == '-')
            || !seen.insert(id)
        {
            return Err(CoreError::Config(
                "workspace contains an invalid or duplicate checkpoint ID".to_string(),
            ));
        }
    }
    Ok(())
}

pub fn template_files_for(template_id: &str) -> Vec<TemplateFile> {
    match template_id {
        "coding" => vec![
            TemplateFile {
                path: "README.md".to_string(),
                content: "# Agent workspace\n\nThis folder is scoped for a coding agent. Keep source files, task notes, and handoff context inside this path.\n".to_string(),
            },
            TemplateFile {
                path: "memory/tasks.md".to_string(),
                content: "# Tasks\n\n- [ ] Define the next task.\n".to_string(),
            },
            TemplateFile {
                path: "memory/decisions.md".to_string(),
                content: "# Decisions\n\nRecord important choices here.\n".to_string(),
            },
            TemplateFile {
                path: "memory/handoff.md".to_string(),
                content: "# Handoff\n\nAdd status notes before changing agents or sessions.\n".to_string(),
            },
        ],
        "research" => vec![
            TemplateFile {
                path: "README.md".to_string(),
                content: "# Research workspace\n\nUse this folder for source material, notes, and explicit research outputs.\n".to_string(),
            },
            TemplateFile {
                path: "memory/questions.md".to_string(),
                content: "# Questions\n\n- What needs to be answered?\n".to_string(),
            },
            TemplateFile {
                path: "memory/sources.md".to_string(),
                content: "# Sources\n\nList files, links, and citations here.\n".to_string(),
            },
            TemplateFile {
                path: "memory/summary.md".to_string(),
                content: "# Summary\n\nWrite concise findings here.\n".to_string(),
            },
        ],
        "writing" => vec![
            TemplateFile {
                path: "README.md".to_string(),
                content: "# Writing workspace\n\nKeep drafts, outlines, notes, and reference material here.\n".to_string(),
            },
            TemplateFile {
                path: "memory/outline.md".to_string(),
                content: "# Outline\n\n- [ ] Section 1\n".to_string(),
            },
            TemplateFile {
                path: "memory/notes.md".to_string(),
                content: "# Notes\n\nWrite notes here.\n".to_string(),
            },
        ],
        "data-analysis" => vec![
            TemplateFile {
                path: "README.md".to_string(),
                content: "# Data analysis workspace\n\nKeep inputs, derived outputs, and run notes inside this storage scope.\n".to_string(),
            },
            TemplateFile {
                path: "memory/datasets.md".to_string(),
                content: "# Datasets\n\nDescribe inputs and freshness here.\n".to_string(),
            },
            TemplateFile {
                path: "memory/observations.md".to_string(),
                content: "# Observations\n\nRecord findings and caveats here.\n".to_string(),
            },
            TemplateFile {
                path: "memory/runbook.md".to_string(),
                content: "# Runbook\n\nDocument repeatable analysis steps here.\n".to_string(),
            },
        ],
        "admin" => vec![
            TemplateFile {
                path: "README.md".to_string(),
                content: "# Admin workspace\n\nUse this folder for administrative tasks, scripts, and configurations.\n".to_string(),
            },
        ],
        _ => vec![
            TemplateFile {
                path: "README.md".to_string(),
                content: "# Workspace\n\nCustom workspace folder.\n".to_string(),
            },
        ],
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WorkspacesFile {
    schema_version: u32,
    #[serde(default)]
    revision: u64,
    workspaces: Vec<WorkspaceRecord>,
}

pub struct WorkspaceRegistry {
    dir: PathBuf,
    // Protects both cache and file operations — serializes all read-modify-write cycles
    file_lock: Mutex<()>,
    // In-memory cache, always consistent with file because file_lock serializes access
    cache: Mutex<HashMap<String, WorkspaceRecord>>,
    // Last-known file revision for compare-before-write
    cached_revision: Mutex<u64>,
}

impl WorkspaceRegistry {
    pub fn new(config_dir: &Path) -> Self {
        Self {
            dir: config_dir.to_path_buf(),
            file_lock: Mutex::new(()),
            cache: Mutex::new(HashMap::new()),
            cached_revision: Mutex::new(0),
        }
    }

    fn path(&self) -> PathBuf {
        self.dir.join(WORKSPACES_FILE)
    }

    /// Acquire the lock used by the desktop workspace command transaction.
    /// Separate registry instances that point at the same directory contend on
    /// the same OS file lock.
    pub fn acquire_mutation_lock(&self) -> Result<WorkspaceMutationGuard> {
        fs::create_dir_all(&self.dir)
            .map_err(|e| CoreError::Config(format!("failed to create workspace directory: {e}")))?;
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(self.dir.join(WORKSPACE_MUTATION_LOCK_FILE))
            .map_err(|e| {
                CoreError::Config(format!("failed to open workspace mutation lock: {e}"))
            })?;
        file.lock_exclusive().map_err(|e| {
            CoreError::Config(format!(
                "failed to lock workspace mutation transaction: {e}"
            ))
        })?;
        Ok(WorkspaceMutationGuard { _file: file })
    }

    fn acquire_disk_lock(&self) -> Result<File> {
        fs::create_dir_all(&self.dir)
            .map_err(|e| CoreError::Config(format!("failed to create workspace directory: {e}")))?;
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(self.dir.join("workspaces.lock"))
            .map_err(|e| CoreError::Config(format!("failed to open workspace lock: {e}")))?;
        file.lock_exclusive()
            .map_err(|e| CoreError::Config(format!("failed to lock workspace registry: {e}")))?;
        Ok(file)
    }

    fn load_raw(&self) -> Result<WorkspacesFile> {
        let path = self.path();
        if !path.exists() {
            return Ok(WorkspacesFile {
                schema_version: WORKSPACES_SCHEMA_VERSION,
                revision: 0,
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

    fn sync_cache(&self, file: &WorkspacesFile) -> Result<()> {
        let mut cache = self
            .cache
            .lock()
            .map_err(|e| CoreError::Config(format!("workspace registry cache poisoned: {e}")))?;
        cache.clear();
        for ws in &file.workspaces {
            cache.insert(ws.id.clone(), ws.clone());
        }
        *self.cached_revision.lock().map_err(|e| {
            CoreError::Config(format!("workspace registry revision poisoned: {e}"))
        })? = file.revision;
        Ok(())
    }

    pub fn load_all(&self) -> Result<Vec<WorkspaceRecord>> {
        let _lock = self
            .file_lock
            .lock()
            .map_err(|e| CoreError::Config(format!("workspace registry lock poisoned: {e}")))?;
        let _disk_lock = self.acquire_disk_lock()?;
        let file = self.load_raw()?;
        self.sync_cache(&file)?;
        Ok(file.workspaces)
    }

    pub fn find_by_id(&self, id: &str) -> Result<Option<WorkspaceRecord>> {
        Ok(self
            .load_all()?
            .into_iter()
            .find(|workspace| workspace.id == id))
    }

    pub fn create(&self, workspace: &WorkspaceRecord) -> Result<()> {
        validate_workspace_metadata(workspace)?;
        let _lock = self
            .file_lock
            .lock()
            .map_err(|e| CoreError::Config(format!("workspace registry lock poisoned: {e}")))?;
        let _disk_lock = self.acquire_disk_lock()?;
        let mut file = self.load_raw()?;
        if file.revision != *self.cached_revision.lock().unwrap() {
            return Err(CoreError::Config(
                "workspace registry file changed since last load; reload and retry".to_string(),
            ));
        }
        if file.workspaces.iter().any(|w| w.id == workspace.id) {
            return Err(CoreError::Config(format!(
                "workspace '{}' already exists",
                workspace.id
            )));
        }
        if file
            .workspaces
            .iter()
            .any(|w| w.storage_id == workspace.storage_id && w.name == workspace.name)
        {
            return Err(CoreError::Config(format!(
                "workspace name '{}' already exists in this storage",
                workspace.name
            )));
        }
        file.workspaces.push(workspace.clone());
        file.revision = file.revision.saturating_add(1);
        self.save_raw(&file)?;
        self.sync_cache(&file)?;
        Ok(())
    }

    pub fn update(&self, workspace: &WorkspaceRecord) -> Result<()> {
        validate_workspace_metadata(workspace)?;
        let _lock = self
            .file_lock
            .lock()
            .map_err(|e| CoreError::Config(format!("workspace registry lock poisoned: {e}")))?;
        let _disk_lock = self.acquire_disk_lock()?;
        let mut file = self.load_raw()?;
        if file.revision != *self.cached_revision.lock().unwrap() {
            return Err(CoreError::Config(
                "workspace registry file changed since last load; reload and retry".to_string(),
            ));
        }
        let idx = file
            .workspaces
            .iter()
            .position(|w| w.id == workspace.id)
            .ok_or_else(|| CoreError::Config(format!("workspace '{}' not found", workspace.id)))?;
        if file.workspaces.iter().any(|w| {
            w.id != workspace.id && w.storage_id == workspace.storage_id && w.name == workspace.name
        }) {
            return Err(CoreError::Config(format!(
                "workspace name '{}' already exists in this storage",
                workspace.name
            )));
        }
        file.workspaces[idx] = workspace.clone();
        file.revision = file.revision.saturating_add(1);
        self.save_raw(&file)?;
        self.sync_cache(&file)?;
        Ok(())
    }

    pub fn delete(&self, id: &str) -> Result<()> {
        let _lock = self
            .file_lock
            .lock()
            .map_err(|e| CoreError::Config(format!("workspace registry lock poisoned: {e}")))?;
        let _disk_lock = self.acquire_disk_lock()?;
        let mut file = self.load_raw()?;
        if file.revision != *self.cached_revision.lock().unwrap() {
            return Err(CoreError::Config(
                "workspace registry file changed since last load; reload and retry".to_string(),
            ));
        }
        let len_before = file.workspaces.len();
        file.workspaces.retain(|w| w.id != id);
        if file.workspaces.len() == len_before {
            return Err(CoreError::Config(format!("workspace '{}' not found", id)));
        }
        file.revision = file.revision.saturating_add(1);
        self.save_raw(&file)?;
        self.sync_cache(&file)?;
        Ok(())
    }

    pub fn import_legacy(&self, legacy_workspaces: Vec<WorkspaceRecord>) -> Result<usize> {
        let _lock = self
            .file_lock
            .lock()
            .map_err(|e| CoreError::Config(format!("workspace registry lock poisoned: {e}")))?;
        let _disk_lock = self.acquire_disk_lock()?;
        let mut file = self.load_raw()?;
        let mut imported = 0;
        for ws in legacy_workspaces {
            validate_workspace_metadata(&ws)?;
            if file.workspaces.iter().any(|w| w.id == ws.id) {
                continue;
            }
            if file
                .workspaces
                .iter()
                .any(|w| w.storage_id == ws.storage_id && w.name == ws.name)
            {
                return Err(CoreError::Config(format!(
                    "workspace name '{}' already exists in this storage",
                    ws.name
                )));
            }
            file.workspaces.push(ws);
            imported += 1;
        }
        if imported > 0 {
            file.revision = file.revision.saturating_add(1);
            self.save_raw(&file)?;
            self.sync_cache(&file)?;
        }
        Ok(imported)
    }

    pub fn replace_all(&self, workspaces: Vec<WorkspaceRecord>) -> Result<()> {
        for workspace in &workspaces {
            validate_workspace_metadata(workspace)?;
        }
        let _lock = self
            .file_lock
            .lock()
            .map_err(|e| CoreError::Config(format!("workspace registry lock poisoned: {e}")))?;
        let _disk_lock = self.acquire_disk_lock()?;
        let current = self.load_raw()?;
        let file = WorkspacesFile {
            schema_version: WORKSPACES_SCHEMA_VERSION,
            revision: current.revision.saturating_add(1),
            workspaces,
        };
        self.save_raw(&file)?;
        self.sync_cache(&file)?;
        Ok(())
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
            schema_version: WORKSPACE_RECORD_SCHEMA_VERSION,
            storage_id: "local".to_string(),
            name: format!("Workspace {id}"),
            root_path: format!("/workspaces/{id}"),
            template_id: "coding".to_string(),
            access_profile: "read_write".to_string(),
            policy_rule_id: Some(format!("workspace:{id}")),
            created_at: "2025-01-01T00:00:00Z".to_string(),
            updated_at: "2025-01-01T00:00:00Z".to_string(),
            memory_files: memory_files_for("coding"),
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

    #[test]
    fn legacy_file_without_revision_or_profile_loads_safely() {
        let dir = tempfile::tempdir().expect("temp dir");
        fs::write(
            dir.path().join(WORKSPACES_FILE),
            r#"{
              "schemaVersion": 1,
              "workspaces": [{
                "id": "legacy",
                "storageId": "local",
                "name": "Legacy",
                "rootPath": "/legacy",
                "templateId": "coding",
                "createdAt": "2025-01-01T00:00:00Z",
                "updatedAt": "2025-01-01T00:00:00Z",
                "memoryFiles": [],
                "checkpointIds": []
              }]
            }"#,
        )
        .expect("write legacy fixture");

        let registry = WorkspaceRegistry::new(dir.path());
        let loaded = registry.load_all().expect("load legacy fixture");
        assert_eq!(loaded[0].access_profile, "read_only");
        assert_eq!(loaded[0].schema_version, WORKSPACE_RECORD_SCHEMA_VERSION);
    }

    #[test]
    fn data_analysis_template_matches_memory_plan() {
        assert_eq!(
            memory_files_for("data-analysis"),
            vec![
                "memory/datasets.md",
                "memory/observations.md",
                "memory/runbook.md"
            ]
        );
        let paths = template_files_for("data-analysis")
            .into_iter()
            .map(|file| file.path)
            .collect::<Vec<_>>();
        assert!(paths.contains(&"README.md".to_string()));
        for memory_file in memory_files_for("data-analysis") {
            assert!(paths.contains(&memory_file));
        }
    }

    #[test]
    fn rejects_untrusted_memory_paths_and_excessive_checkpoints() {
        let (registry, _dir) = test_registry();
        let mut workspace = make_ws("unsafe");
        workspace.memory_files = vec!["../secret".to_string()];
        assert!(registry.create(&workspace).is_err());

        workspace.memory_files = memory_files_for("coding");
        workspace.checkpoint_ids = (0..=MAX_CHECKPOINT_IDS)
            .map(|index| format!("checkpoint-{index}"))
            .collect();
        assert!(registry.create(&workspace).is_err());
    }

    #[test]
    fn mutation_lock_serializes_registry_instances() {
        use std::sync::mpsc;
        use std::time::Duration;

        let dir = tempfile::tempdir().expect("temp dir");
        let first = WorkspaceRegistry::new(dir.path());
        let second = WorkspaceRegistry::new(dir.path());
        let guard = first.acquire_mutation_lock().expect("first lock");
        let (sender, receiver) = mpsc::channel();
        let thread = std::thread::spawn(move || {
            let _guard = second.acquire_mutation_lock().expect("second lock");
            sender.send(()).expect("notify acquired");
        });
        assert!(receiver.recv_timeout(Duration::from_millis(50)).is_err());
        drop(guard);
        receiver
            .recv_timeout(Duration::from_secs(2))
            .expect("second instance should acquire after release");
        thread.join().expect("lock thread");
    }

    #[test]
    fn two_instances_detect_and_recover_from_stale_revision() {
        let dir = tempfile::tempdir().expect("temp dir");
        let first = WorkspaceRegistry::new(dir.path());
        let second = WorkspaceRegistry::new(dir.path());
        first.load_all().expect("prime first");
        second.load_all().expect("prime second");

        first.create(&make_ws("first")).expect("first create");
        let stale = second.create(&make_ws("second")).unwrap_err();
        assert!(stale.to_string().contains("reload and retry"));

        second.load_all().expect("reload second");
        second.create(&make_ws("second")).expect("retry create");
        assert_eq!(first.load_all().expect("final list").len(), 2);
    }
}
