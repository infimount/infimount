import { createDirectory, listEntries, readFile, writeFile } from "@/lib/api";
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

export interface AgentWorkspace {
  id: string;
  storageId: string;
  name: string;
  rootPath: string;
  templateId: AgentWorkspaceTemplateId;
  createdAt: string;
  updatedAt: string;
  policy: McpStoragePolicy;
  memoryFiles: string[];
  checkpointIds: string[];
}

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

const WORKSPACES_STORAGE_KEY = "infimount:agent-workspaces:v1";
const CHECKPOINTS_STORAGE_KEY = "infimount:agent-workspace-checkpoints:v1";
const WORKSPACE_MANIFEST_PATH = ".infimount/workspace.json";
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
      { path: "memory/observations.md", content: "# Observations\n\nRecord findings and caveats here.\n" },
      { path: "memory/runbook.md", content: "# Runbook\n\nDocument repeatable analysis steps here.\n" },
    ],
  },
];

export function listAgentWorkspaces(): AgentWorkspace[] {
  if (typeof window === "undefined") return [];
  try {
    const parsed = JSON.parse(window.localStorage.getItem(WORKSPACES_STORAGE_KEY) ?? "[]");
    if (!Array.isArray(parsed)) return [];
    return parsed.filter(isWorkspace).sort((a, b) => b.updatedAt.localeCompare(a.updatedAt));
  } catch {
    return [];
  }
}

export function saveAgentWorkspaces(workspaces: AgentWorkspace[]) {
  if (typeof window === "undefined") return;
  window.localStorage.setItem(WORKSPACES_STORAGE_KEY, JSON.stringify(workspaces));
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

  // Generate ID before policy — reject all normalized-root variants early
  const rawRoot = rootPath || defaultWorkspacePath(name);
  const normalizedRoot = normalizeWorkspacePath(rawRoot);
  if (normalizedRoot === "/" || normalizedRoot === "." || normalizedRoot === "..") {
    throw new Error("Workspace root path must be a non-root directory");
  }

  // Build policy with workspace ID (not the other way around)
  const policy = buildWorkspacePolicy(normalizedRoot, currentPolicy, workspaceId);

  const now = new Date().toISOString();
  const workspace: AgentWorkspace = {
    id: workspaceId,
    storageId,
    name: name.trim(),
    rootPath: normalizedRoot,
    templateId,
    createdAt: now,
    updatedAt: now,
    policy,
    memoryFiles: template.memoryFiles,
    checkpointIds: [],
  };

  await createWorkspaceDirectories(
    storageId,
    normalizedRoot,
    template.files.map((file) => file.path),
  );
  await createDirectory(storageId, joinWorkspacePath(normalizedRoot, CHECKPOINTS_DIR)).catch(
    () => undefined,
  );

  for (const file of template.files) {
    await writeTextFile(storageId, joinWorkspacePath(normalizedRoot, file.path), file.content);
  }
  await writeWorkspaceManifest(workspace);

  if (updatePolicy) {
    await updatePolicy(policy);
  }

  // Preserve all other workspaces, only add/update the target workspace
  const others = listAgentWorkspaces().filter((item) => item.id !== workspace.id);
  saveAgentWorkspaces([...others, workspace]);
  appendActivityLogEvent({
    type: "workspace_created",
    operation: "workspace",
    sourceId: storageId,
    workspaceId: workspace.id,
    message: `Created agent workspace ${workspace.name}`,
    summary: { rootPath: normalizedRoot, templateId, policyScoped: Boolean(updatePolicy) },
  });

  return workspace;
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
  const updatedAt = checkpoint.createdAt;
  saveAgentWorkspaces(
    listAgentWorkspaces().map((item) =>
      item.id === workspace.id
        ? {
            ...item,
            updatedAt,
            checkpointIds: [
              checkpoint.id,
              ...item.checkpointIds.filter((id) => id !== checkpoint.id),
            ],
          }
        : item,
    ),
  );
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

  // Decode percent-encoded sequences that map to path separators/dots
  const decoded = collapsed
    .replace(/%2e/gi, ".")
    .replace(/%2f/gi, "/")
    .replace(/%5c/gi, "/");

  // Re-collapse after decoding (handles %2e%2e -> .., %2f -> /, etc.)
  const segments: string[] = [];
  for (const segment of decoded.split("/")) {
    if (segment === "" || segment === ".") continue;
    if (segment === "..") { segments.pop(); continue; }
    segments.push(segment);
  }

  const normalized = segments.length === 0 ? "/" : `/${segments.join("/")}`;

  // Reject normalized root variants matching Rust semantics
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
  // Preserve all rules except those for this specific workspace
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
      // Only remove rules belonging to this specific workspace
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

async function createWorkspaceDirectories(
  storageId: string,
  rootPath: string,
  files: string[],
): Promise<void> {
  const directories = new Set<string>([
    rootPath,
    joinWorkspacePath(rootPath, "memory"),
    joinWorkspacePath(rootPath, ".infimount"),
    joinWorkspacePath(rootPath, CHECKPOINTS_DIR),
  ]);
  for (const file of files) {
    const segments = file.split("/").filter(Boolean);
    for (let index = 1; index < segments.length; index += 1) {
      directories.add(joinWorkspacePath(rootPath, segments.slice(0, index).join("/")));
    }
  }

  for (const directory of Array.from(directories).sort((a, b) => a.length - b.length)) {
    await createDirectory(storageId, directory).catch(() => undefined);
  }
}

async function writeWorkspaceManifest(workspace: AgentWorkspace): Promise<void> {
  const manifest = {
    kind: "infimount-agent-workspace",
    version: 1,
    workspace: {
      id: workspace.id,
      name: workspace.name,
      rootPath: workspace.rootPath,
      templateId: workspace.templateId,
      memoryFiles: workspace.memoryFiles,
      policy: workspace.policy,
      createdAt: workspace.createdAt,
      updatedAt: workspace.updatedAt,
    },
  };
  await writeJsonFile(
    workspace.storageId,
    joinWorkspacePath(workspace.rootPath, WORKSPACE_MANIFEST_PATH),
    manifest,
  );
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

function isWorkspace(value: unknown): value is AgentWorkspace {
  if (!value || typeof value !== "object") return false;
  const item = value as AgentWorkspace;
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
