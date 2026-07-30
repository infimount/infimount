import { createDirectory, listEntries, readFile, writeFile } from "@/lib/api";
import {
  listWorkspaces,
  createWorkspaceAtomic,
  updateWorkspace as apiUpdateWorkspace,
  deleteWorkspace as apiDeleteWorkspace,
  importLegacyWorkspaces as apiImportLegacy,
  type WorkspaceRecord,
} from "@/lib/api";
import { appendActivityLogEvent } from "@/lib/activityLog";
import type { McpStoragePolicy } from "@/types/storage";

export type AgentWorkspaceTemplateId = "coding" | "research" | "data-analysis";

export interface AgentWorkspaceTemplate {
  id: AgentWorkspaceTemplateId;
  name: string;
  description: string;
  memoryFiles: string[];
  files: Array<{ path: string; content: string }>;
}

export type AgentWorkspace = WorkspaceRecord;

export interface AgentWorkspaceCheckpoint {
  id: string;
  workspaceId: string;
  createdAt: string;
  label: string;
  manifestPath: string;
  memoryFiles: Array<{ path: string; content: string }>;
}

export interface CreateAgentWorkspaceInput {
  storageId: string;
  workspaceId?: string;
  name: string;
  rootPath: string;
  templateId: AgentWorkspaceTemplateId;
  currentPolicy?: McpStoragePolicy;
  updatePolicy?: (policy: McpStoragePolicy) => Promise<void>;
}

const LEGACY_STORAGE_KEY = "infimount:agent-workspaces:v1";
const CHECKPOINTS_STORAGE_KEY = "infimount:agent-workspace-checkpoints:v1";
const CHECKPOINTS_DIR = ".infimount/checkpoints";

export const AGENT_WORKSPACE_TEMPLATES: AgentWorkspaceTemplate[] = [
  {
    id: "coding",
    name: "Coding agent",
    description: "A scoped project folder with tasks, decisions, and handoff notes.",
    memoryFiles: ["memory/tasks.md", "memory/decisions.md", "memory/handoff.md"],
    files: [
      {
        path: "README.md",
        content:
          "# Agent workspace\n\nThis folder is scoped for a coding agent. Keep source files, task notes, and handoff context inside this path.\n",
      },
      { path: "memory/tasks.md", content: "# Tasks\n\n- [ ] Define the next task.\n" },
      { path: "memory/decisions.md", content: "# Decisions\n\nRecord important choices here.\n" },
      {
        path: "memory/handoff.md",
        content: "# Handoff\n\nAdd status notes before changing agents or sessions.\n",
      },
    ],
  },
  {
    id: "research",
    name: "Research agent",
    description: "A quiet place for sources, summaries, questions, and synthesis.",
    memoryFiles: ["memory/questions.md", "memory/sources.md", "memory/summary.md"],
    files: [
      {
        path: "README.md",
        content:
          "# Research workspace\n\nUse this folder for source material, notes, and explicit research outputs.\n",
      },
      { path: "memory/questions.md", content: "# Questions\n\n- What needs to be answered?\n" },
      { path: "memory/sources.md", content: "# Sources\n\nList files, links, and citations here.\n" },
      { path: "memory/summary.md", content: "# Summary\n\nWrite concise findings here.\n" },
    ],
  },
  {
    id: "data-analysis",
    name: "Data analysis agent",
    description: "A scoped area for inputs, notebooks, outputs, and observations.",
    memoryFiles: ["memory/datasets.md", "memory/observations.md", "memory/runbook.md"],
    files: [
      {
        path: "README.md",
        content:
          "# Data analysis workspace\n\nKeep inputs, derived outputs, and run notes inside this storage scope.\n",
      },
      { path: "memory/datasets.md", content: "# Datasets\n\nDescribe inputs and freshness here.\n" },
      {
        path: "memory/observations.md",
        content: "# Observations\n\nRecord findings and caveats here.\n",
      },
      {
        path: "memory/runbook.md",
        content: "# Runbook\n\nDocument repeatable analysis steps here.\n",
      },
    ],
  },
];

export async function listAgentWorkspaces(): Promise<AgentWorkspace[]> {
  const workspaces = await listWorkspaces();
  return workspaces.sort((a, b) => b.updatedAt.localeCompare(a.updatedAt));
}

export function listAgentWorkspaceCheckpoints(workspaceId: string): AgentWorkspaceCheckpoint[] {
  return readCheckpoints()
    .filter((checkpoint) => checkpoint.workspaceId === workspaceId)
    .sort((a, b) => b.createdAt.localeCompare(a.createdAt));
}

export async function createAgentWorkspace({
  storageId,
  workspaceId: externalWorkspaceId,
  name,
  rootPath,
  templateId,
  currentPolicy,
  updatePolicy,
}: CreateAgentWorkspaceInput): Promise<AgentWorkspace> {
  const template = getWorkspaceTemplate(templateId);
  const workspaceId = externalWorkspaceId ?? `workspace-${Date.now()}-${Math.random().toString(36).slice(2, 9)}`;

  const rawRoot = rootPath || defaultWorkspacePath(name);
  const normalizedRoot = normalizeWorkspacePath(rawRoot);
  if (normalizedRoot === "/" || normalizedRoot === "." || normalizedRoot === "..") {
    throw new Error("Workspace root path must be a non-root directory");
  }

  const policy = buildWorkspacePolicy(normalizedRoot, currentPolicy, workspaceId);

  const result = await createWorkspaceAtomic({
    id: workspaceId,
    storageId,
    name: name.trim(),
    rootPath: normalizedRoot,
    templateId,
    memoryFiles: template.memoryFiles,
    templateFiles: template.files.map((f) => ({ path: f.path, content: f.content })),
    updatePolicy: updatePolicy ? policy : undefined,
  });

  if (result.rollbackErrors.length > 0) {
    console.warn("Workspace creation rollback completed with errors:", result.rollbackErrors);
  }

  appendActivityLogEvent({
    type: "workspace_created",
    operation: "workspace",
    sourceId: storageId,
    workspaceId: result.workspace.id,
    message: `Created agent workspace ${result.workspace.name}`,
    summary: { rootPath: normalizedRoot, templateId, policyScoped: result.policyUpdated },
  });

  return result.workspace;
}

export async function listWorkspaceMemoryFiles(workspace: AgentWorkspace): Promise<string[]> {
  const entries = await listEntries(
    workspace.storageId,
    joinWorkspacePath(workspace.rootPath, "memory"),
  );
  const paths = entries
    .filter((entry) => !entry.is_dir)
    .map((entry) => joinRelativePath("memory", entry.name))
    .sort((a, b) => a.localeCompare(b));
  return paths.length > 0 ? paths : workspace.memoryFiles;
}

export async function readWorkspaceMemoryFile(
  workspace: AgentWorkspace,
  relativePath: string,
): Promise<string> {
  const data = await readFile(workspace.storageId, joinWorkspacePath(workspace.rootPath, relativePath));
  return new TextDecoder().decode(data);
}

export async function appendWorkspaceMemory(
  workspace: AgentWorkspace,
  relativePath: string,
  note: string,
): Promise<string> {
  const current = await readWorkspaceMemoryFile(workspace, relativePath).catch(() => "");
  const separator = current.endsWith("\n") || current.length === 0 ? "" : "\n";
  const next = `${current}${separator}${note.trim()}\n`;
  await writeTextFile(workspace.storageId, joinWorkspacePath(workspace.rootPath, relativePath), next);
  appendActivityLogEvent({
    type: "workspace_memory_appended",
    operation: "workspace",
    sourceId: workspace.storageId,
    workspaceId: workspace.id,
    message: `Updated ${relativePath}`,
    summary: { rootPath: workspace.rootPath, relativePath },
  });
  return next;
}

export async function createWorkspaceCheckpoint(
  workspace: AgentWorkspace,
  label?: string,
): Promise<AgentWorkspaceCheckpoint> {
  const checkpointId = `checkpoint-${Date.now()}-${Math.random().toString(36).slice(2, 9)}`;
  const memoryFiles = await Promise.all(
    workspace.memoryFiles.map(async (path) => ({
      path,
      content: await readWorkspaceMemoryFile(workspace, path).catch(() => ""),
    })),
  );
  const checkpoint: AgentWorkspaceCheckpoint = {
    id: checkpointId,
    workspaceId: workspace.id,
    createdAt: new Date().toISOString(),
    label: label?.trim() || "Memory checkpoint",
    manifestPath: workspaceCheckpointManifestPath(checkpointId),
    memoryFiles,
  };

  await writeCheckpointManifest(workspace, checkpoint);

  const checkpoints = [checkpoint, ...readCheckpoints()].slice(0, 200);
  saveCheckpoints(checkpoints);

  await apiUpdateWorkspace({
    id: workspace.id,
    checkpointIds: [
      checkpoint.id,
      ...workspace.checkpointIds.filter((id) => id !== checkpoint.id),
    ],
  });

  appendActivityLogEvent({
    type: "workspace_checkpoint_created",
    operation: "workspace",
    sourceId: workspace.storageId,
    workspaceId: workspace.id,
    message: `Checkpointed ${workspace.name}`,
    summary: {
      checkpointId: checkpoint.id,
      fileCount: memoryFiles.length,
      manifestPath: checkpoint.manifestPath,
    },
  });
  return checkpoint;
}

export async function restoreWorkspaceMemoryCheckpoint(
  workspace: AgentWorkspace,
  checkpointId: string,
): Promise<void> {
  const checkpoint = await loadCheckpoint(workspace, checkpointId);
  if (!checkpoint) throw new Error("Checkpoint not found");

  for (const file of checkpoint.memoryFiles) {
    await writeTextFile(workspace.storageId, joinWorkspacePath(workspace.rootPath, file.path), file.content);
  }
  appendActivityLogEvent({
    type: "workspace_checkpoint_restored",
    operation: "workspace",
    sourceId: workspace.storageId,
    workspaceId: workspace.id,
    message: `Restored ${checkpoint.label}`,
    summary: {
      checkpointId: checkpoint.id,
      fileCount: checkpoint.memoryFiles.length,
      manifestPath: checkpoint.manifestPath,
    },
  });
}

export function getWorkspaceTemplate(templateId: AgentWorkspaceTemplateId): AgentWorkspaceTemplate {
  return (
    AGENT_WORKSPACE_TEMPLATES.find((template) => template.id === templateId) ??
    AGENT_WORKSPACE_TEMPLATES[0]
  );
}

export function defaultWorkspacePath(name: string): string {
  return `/agent-workspaces/${slugify(name || "workspace")}`;
}

export function normalizeWorkspacePath(path: string): string {
  const trimmed = path.trim();
  const withRoot = trimmed.startsWith("/") ? trimmed : `/${trimmed}`;
  const collapsed = withRoot.replace(/\\/g, "/").replace(/\/+/g, "/");

  const decoded = collapsed
    .replace(/%2e/gi, ".")
    .replace(/%2f/gi, "/")
    .replace(/%5c/gi, "/");

  const segments: string[] = [];
  for (const segment of decoded.split("/")) {
    if (segment === "" || segment === ".") continue;
    if (segment === "..") { segments.pop(); continue; }
    segments.push(segment);
  }

  const normalized = segments.length === 0 ? "/" : `/${segments.join("/")}`;

  if (normalized === "/") {
    throw new Error("Workspace root path must not be or resolve to '/'");
  }
  if (normalized === "." || normalized === "..") {
    throw new Error("Workspace root path must not be '.' or '..'");
  }
  if (/^\.+$/.test(normalized.replace(/\//g, ""))) {
    throw new Error("Workspace root path must not consist solely of dots");
  }

  return normalized;
}

export function joinWorkspacePath(rootPath: string, relativePath: string): string {
  const root = normalizeWorkspacePath(rootPath);
  const relative = relativePath.replace(/^\/+/, "").replace(/\/+/g, "/");
  if (!relative) return root;
  if (root === "/") return `/${relative}`;
  return `${root}/${relative}`;
}

export function workspaceCheckpointManifestPath(checkpointId: string): string {
  return `${CHECKPOINTS_DIR}/${checkpointId}.json`;
}

export function buildWorkspacePolicy(
  rootPath: string,
  currentPolicy?: McpStoragePolicy,
  workspaceId?: string,
): McpStoragePolicy {
  if (!workspaceId) {
    throw new Error("workspaceId is required to build workspace policy");
  }
  const trimmed = rootPath.trim();
  if (!trimmed) {
    throw new Error("Workspace root path must not be empty");
  }
  const normalizedRoot = normalizeWorkspacePath(rootPath);
  if (normalizedRoot === "/") {
    throw new Error("Workspace root path must not be or resolve to '/'");
  }
  const scopedRoot = `${normalizedRoot}/`;
  const ruleId = `ws:${workspaceId}`;

  const existingRules = currentPolicy?.rules ?? [];
  const otherRules = existingRules.filter(
    (r) => r.source.kind !== "workspace" || r.source.workspace_id !== workspaceId,
  );

  return {
    version: 2,
    default_access: currentPolicy?.default_access ?? "none",
    rules: [
      ...otherRules,
      {
        id: ruleId,
        prefix: scopedRoot,
        access: "read_only",
        source: { kind: "workspace", workspace_id: workspaceId },
      },
    ],
    denied_paths: currentPolicy?.denied_paths ?? [],
    confirmation_rules: currentPolicy?.confirmation_rules ?? {
      require_for_write: true,
      require_for_overwrite: true,
      require_for_delete: true,
      require_for_version_delete: true,
      require_for_presign: true,
      require_for_cross_storage_copy: true,
    },
  };
}

export function removeWorkspacePolicy(
  currentPolicy: McpStoragePolicy,
  workspaceId: string,
): McpStoragePolicy {
  return {
    ...currentPolicy,
    rules: currentPolicy.rules.filter((r) => {
      if (r.source.kind === "workspace" && r.source.workspace_id === workspaceId) {
        return false;
      }
      if (r.id === `ws:${workspaceId}`) {
        return false;
      }
      return true;
    }),
  };
}

export async function deleteAgentWorkspace(id: string): Promise<void> {
  await apiDeleteWorkspace(id);
}

export async function migrateLegacyWorkspaces(): Promise<number> {
  if (typeof window === "undefined") return 0;
  const raw = window.localStorage.getItem(LEGACY_STORAGE_KEY);
  if (!raw) return 0;

  let legacy: unknown[];
  try {
    legacy = JSON.parse(raw);
    if (!Array.isArray(legacy)) return 0;
  } catch {
    return 0;
  }

  const records: WorkspaceRecord[] = legacy
    .filter(isLegacyWorkspace)
    .map((item) => ({
      id: item.id,
      storageId: item.storageId,
      name: item.name,
      rootPath: item.rootPath,
      templateId: item.templateId,
      createdAt: item.createdAt,
      updatedAt: item.updatedAt,
      memoryFiles: item.memoryFiles || [],
      checkpointIds: item.checkpointIds || [],
    }));

  if (records.length === 0) return 0;

  const imported = await apiImportLegacy({ workspaces: records });
  if (imported > 0) {
    window.localStorage.removeItem(LEGACY_STORAGE_KEY);
  }
  return imported;
}

async function writeCheckpointManifest(
  workspace: AgentWorkspace,
  checkpoint: AgentWorkspaceCheckpoint,
): Promise<void> {
  await createDirectory(workspace.storageId, joinWorkspacePath(workspace.rootPath, CHECKPOINTS_DIR)).catch(
    () => undefined,
  );
  await writeJsonFile(
    workspace.storageId,
    joinWorkspacePath(workspace.rootPath, checkpoint.manifestPath),
    {
      kind: "infimount-agent-workspace-checkpoint",
      version: 1,
      checkpoint,
    },
  );
}

async function loadCheckpoint(
  workspace: AgentWorkspace,
  checkpointId: string,
): Promise<AgentWorkspaceCheckpoint | null> {
  const local = readCheckpoints().find(
    (item) => item.workspaceId === workspace.id && item.id === checkpointId,
  );
  if (local) return local;

  const manifestPath = workspaceCheckpointManifestPath(checkpointId);
  try {
    const raw = await readWorkspaceTextFile(workspace, manifestPath);
    const parsed = JSON.parse(raw) as { checkpoint?: unknown };
    if (isCheckpoint(parsed.checkpoint)) return parsed.checkpoint;
  } catch {
    return null;
  }
  return null;
}

async function readWorkspaceTextFile(
  workspace: AgentWorkspace,
  relativePath: string,
): Promise<string> {
  const data = await readFile(workspace.storageId, joinWorkspacePath(workspace.rootPath, relativePath));
  return new TextDecoder().decode(data);
}

async function writeTextFile(storageId: string, path: string, content: string): Promise<void> {
  await writeFile(storageId, path, new TextEncoder().encode(content));
}

async function writeJsonFile(
  storageId: string,
  path: string,
  value: unknown,
): Promise<void> {
  await writeTextFile(storageId, path, `${JSON.stringify(value, null, 2)}\n`);
}

function readCheckpoints(): AgentWorkspaceCheckpoint[] {
  if (typeof window === "undefined") return [];
  try {
    const parsed = JSON.parse(window.localStorage.getItem(CHECKPOINTS_STORAGE_KEY) ?? "[]");
    if (!Array.isArray(parsed)) return [];
    return parsed.filter(isCheckpoint);
  } catch {
    return [];
  }
}

function saveCheckpoints(checkpoints: AgentWorkspaceCheckpoint[]) {
  if (typeof window === "undefined") return;
  window.localStorage.setItem(CHECKPOINTS_STORAGE_KEY, JSON.stringify(checkpoints));
}

function isCheckpoint(value: unknown): value is AgentWorkspaceCheckpoint {
  if (!value || typeof value !== "object") return false;
  const item = value as AgentWorkspaceCheckpoint;
  return (
    typeof item.id === "string" &&
    typeof item.workspaceId === "string" &&
    typeof item.createdAt === "string" &&
    typeof item.label === "string" &&
    typeof item.manifestPath === "string" &&
    Array.isArray(item.memoryFiles)
  );
}

function isLegacyWorkspace(value: unknown): value is WorkspaceRecord {
  if (!value || typeof value !== "object") return false;
  const item = value as WorkspaceRecord;
  return (
    typeof item.id === "string" &&
    typeof item.storageId === "string" &&
    typeof item.name === "string" &&
    typeof item.rootPath === "string" &&
    typeof item.templateId === "string" &&
    Array.isArray(item.memoryFiles) &&
    Array.isArray(item.checkpointIds)
  );
}

function joinRelativePath(base: string, name: string): string {
  return `${base.replace(/\/+$/g, "")}/${name.replace(/^\/+/, "")}`;
}

function slugify(value: string): string {
  const slug = value
    .trim()
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-+|-+$/g, "");
  return slug || "workspace";
}
