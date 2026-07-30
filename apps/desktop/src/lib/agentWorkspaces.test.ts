import { beforeEach, describe, expect, it, vi } from "vitest";

import {
  appendWorkspaceMemory,
  buildWorkspacePolicy,
  createAgentWorkspace,
  createWorkspaceCheckpoint,
  defaultWorkspacePath,
  listAgentWorkspaceCheckpoints,
  listAgentWorkspaces,
  migrateLegacyWorkspaces,
  normalizeWorkspacePath,
  removeWorkspacePolicy,
  restoreWorkspaceMemoryCheckpoint,
} from "./agentWorkspaces";
import {
  listWorkspaces,
  createWorkspaceAtomic,
  updateWorkspace as apiUpdateWorkspace,
  createDirectory,
  readFile,
  writeFile,
  importLegacyWorkspaces as apiImportLegacyWorkspaces,
  type WorkspaceRecord,
  type CreateWorkspaceAtomicInput,
} from "./api";
import type { McpStoragePolicy } from "@/types/storage";

const policy: McpStoragePolicy = {
  version: 2,
  default_access: "read_write",
  rules: [],
  allowed_paths: [],
  denied_paths: [],
  confirmation_rules: {
    require_for_write: true,
    require_for_overwrite: true,
    require_for_delete: true,
    require_for_version_delete: true,
    require_for_presign: true,
    require_for_cross_storage_copy: true,
  },
};

vi.mock("./api", async (importOriginal) => {
  const actual = await importOriginal<typeof import("./api")>();
  return {
    ...actual,
    listWorkspaces: vi.fn().mockResolvedValue([]),
    createWorkspaceAtomic: vi.fn().mockImplementation(
      async (input: CreateWorkspaceAtomicInput) => {
        const now = new Date().toISOString();
        return {
          workspace: {
            id: `generated-${input.name.toLowerCase().replace(/ /g, "-")}`,
            storageId: input.storageId,
            name: input.name,
            rootPath: input.rootPath,
            templateId: input.templateId,
            createdAt: now,
            updatedAt: now,
            memoryFiles: input.templateId === "coding"
              ? ["memory/tasks.md", "memory/decisions.md", "memory/handoff.md"]
              : [],
            checkpointIds: [],
          },
          policyUpdated: !!input.accessProfile,
          rollbackErrors: [],
        };
      },
    ),
    updateWorkspace: vi.fn().mockResolvedValue({}),
    createDirectory: vi.fn().mockResolvedValue(undefined),
    listEntries: vi.fn().mockResolvedValue([]),
    readFile: vi.fn(),
    writeFile: vi.fn(),
    importLegacyWorkspaces: vi.fn(),
  };
});

describe("agentWorkspaces", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    window.localStorage.clear();
    vi.mocked(createDirectory).mockResolvedValue(undefined);
    vi.mocked(writeFile).mockResolvedValue(undefined);
    vi.mocked(readFile).mockResolvedValue(new TextEncoder().encode("# Tasks\n"));
    (listWorkspaces as ReturnType<typeof vi.fn>).mockResolvedValue([]);
    (apiUpdateWorkspace as ReturnType<typeof vi.fn>).mockResolvedValue({});
  });

  it("creates a workspace, template files, and scoped MCP policy", async () => {
    const workspace = await createAgentWorkspace({
      storageId: "local",
      name: "Coding Workspace",
      rootPath: "agent space",
      templateId: "coding",
      accessProfile: "read_only",
    });

    expect(workspace.rootPath).toBe("/agent space");
    expect(createWorkspaceAtomic).toHaveBeenCalledWith(
      expect.objectContaining({ accessProfile: "read_only" }),
    );
  });

  it("appends memory and checkpoints memory files", async () => {
    (createWorkspaceAtomic as ReturnType<typeof vi.fn>).mockResolvedValue({
      workspace: {
        id: "research-ws",
        storageId: "local",
        name: "Research",
        rootPath: "/research",
        templateId: "research",
        createdAt: "2026-01-01T00:00:00Z",
        updatedAt: "2026-01-01T00:00:00Z",
        memoryFiles: ["memory/questions.md", "memory/sources.md", "memory/summary.md"],
        checkpointIds: [],
      },
      policyUpdated: false,
      rollbackErrors: [],
    });

    const workspace = await createAgentWorkspace({
      storageId: "local",
      name: "Research",
      rootPath: "/research",
      templateId: "research",
    });

    await appendWorkspaceMemory(workspace, "memory/questions.md", "What changed?");
    expect(writeFile).toHaveBeenLastCalledWith(
      "local",
      "/research/memory/questions.md",
      expect.anything(),
    );

    const checkpoint = await createWorkspaceCheckpoint(workspace);
    expect(checkpoint.manifestPath).toMatch(/^\.infimount\/checkpoints\/checkpoint-/);
    const manifestCall = vi
      .mocked(writeFile)
      .mock.calls.find((call) => call[1] === `/research/${checkpoint.manifestPath}`);
    expect(manifestCall).toBeTruthy();
    vi.mocked(readFile).mockResolvedValue(manifestCall?.[2] as Uint8Array);
    await expect(listAgentWorkspaceCheckpoints(workspace)).resolves.toMatchObject([
      { id: checkpoint.id },
    ]);
    expect(writeFile).toHaveBeenCalledWith(
      "local",
      `/research/${checkpoint.manifestPath}`,
      expect.anything(),
    );

    await restoreWorkspaceMemoryCheckpoint(workspace, checkpoint.id);
    expect(writeFile).toHaveBeenCalledWith(
      "local",
      "/research/memory/questions.md",
      expect.anything(),
    );
  });

  it("restores a checkpoint from the OpenDAL workspace manifest when local state is missing", async () => {
    (createWorkspaceAtomic as ReturnType<typeof vi.fn>).mockResolvedValue({
      workspace: {
        id: "research-ws-2",
        storageId: "local",
        name: "Research",
        rootPath: "/research",
        templateId: "research",
        createdAt: "2026-01-01T00:00:00Z",
        updatedAt: "2026-01-01T00:00:00Z",
        memoryFiles: ["memory/questions.md", "memory/sources.md", "memory/summary.md"],
        checkpointIds: [],
      },
      policyUpdated: false,
      rollbackErrors: [],
    });

    const workspace = await createAgentWorkspace({
      storageId: "local",
      name: "Research",
      rootPath: "/research",
      templateId: "research",
    });
    const checkpoint = await createWorkspaceCheckpoint(workspace);
    const manifestCall = vi
      .mocked(writeFile)
      .mock.calls.find((call) => call[1] === `/research/${checkpoint.manifestPath}`);
    expect(manifestCall).toBeTruthy();
    const manifestBytes = manifestCall?.[2] as Uint8Array;

    window.localStorage.setItem(
      "infimount:agent-workspace-checkpoints:v1",
      JSON.stringify([{ id: checkpoint.id, content: "untrusted" }]),
    );
    vi.mocked(readFile).mockResolvedValueOnce(manifestBytes);

    await restoreWorkspaceMemoryCheckpoint(workspace, checkpoint.id);

    expect(window.localStorage.getItem("infimount:agent-workspace-checkpoints:v1")).toBeNull();
    expect(readFile).toHaveBeenCalledWith("local", `/research/${checkpoint.manifestPath}`);
    expect(writeFile).toHaveBeenCalledWith(
      "local",
      "/research/memory/questions.md",
      expect.anything(),
    );
  });

  it("rejects arbitrary memory paths before reading or writing storage", async () => {
    const workspace: WorkspaceRecord = {
      id: "unsafe",
      storageId: "local",
      name: "Unsafe",
      rootPath: "/unsafe",
      templateId: "coding",
      createdAt: "2026-01-01T00:00:00Z",
      updatedAt: "2026-01-01T00:00:00Z",
      memoryFiles: ["../secret"],
      checkpointIds: [],
    };

    await expect(createWorkspaceCheckpoint(workspace)).rejects.toThrow("trusted template");
    await expect(appendWorkspaceMemory(workspace, "../secret", "overwrite")).rejects.toThrow(
      "Unsafe workspace memory path",
    );
    expect(writeFile).not.toHaveBeenCalled();
  });

  it("normalizes paths and builds default workspace paths", () => {
    expect(defaultWorkspacePath("My Workspace!")).toBe("/agent-workspaces/my-workspace");
    expect(normalizeWorkspacePath("//team//agent//")).toBe("/team/agent");
  });

  it("buildWorkspacePolicy rejects root path", () => {
    expect(() => buildWorkspacePolicy("/", policy, "w-1")).toThrow("must not be or resolve to '/'");
    expect(() => buildWorkspacePolicy("", policy, "w-1")).toThrow("must not be empty");
  });

  it("buildWorkspacePolicy requires workspaceId", () => {
    expect(() => buildWorkspacePolicy("/team/agent", policy, undefined as unknown as string)).toThrow(
      "workspaceId is required",
    );
  });

  it("buildWorkspacePolicy creates scoped rule with workspace ID", () => {
    const result = buildWorkspacePolicy("/team/agent", policy, "w-1");
    expect(result.version).toBe(2);
    expect(result.rules).toHaveLength(1);
    expect(result.rules[0]).toMatchObject({
      id: "ws:w-1",
      prefix: "/team/agent/",
      access: "read_only",
      source: { kind: "workspace", workspace_id: "w-1" },
    });
  });

  it("buildWorkspacePolicy preserves other workspaces and non-workspace rules", () => {
    const existingPolicy: McpStoragePolicy = {
      ...policy,
      rules: [
        { id: "manual-rule", prefix: "public", access: "read_write", source: { kind: "manual" } },
        {
          id: "ws:other",
          prefix: "/other-ws/",
          access: "read_only",
          source: { kind: "workspace", workspace_id: "other" },
        },
      ],
    };
    const result = buildWorkspacePolicy("/team/agent", existingPolicy, "w-1");
    expect(result.rules).toHaveLength(3);
    expect(result.rules.find((r) => r.id === "manual-rule")).toBeTruthy();
    expect(result.rules.find((r) => r.id === "ws:other")).toBeTruthy();
    expect(result.rules.find((r) => r.id === "ws:w-1")).toBeTruthy();
  });

  it("buildWorkspacePolicy replaces existing rule for same workspace ID", () => {
    const existingPolicy: McpStoragePolicy = {
      ...policy,
      rules: [
        {
          id: "ws:w-1",
          prefix: "/old-path/",
          access: "read_only",
          source: { kind: "workspace", workspace_id: "w-1" },
        },
      ],
    };
    const result = buildWorkspacePolicy("/new-path", existingPolicy, "w-1");
    expect(result.rules).toHaveLength(1);
    expect(result.rules[0].prefix).toBe("/new-path/");
  });

  it("removeWorkspacePolicy only removes target workspace rules", () => {
    const existingPolicy: McpStoragePolicy = {
      ...policy,
      rules: [
        { id: "manual-1", prefix: "public", access: "read_write", source: { kind: "manual" } },
        {
          id: "ws:w-1",
          prefix: "/team-a/",
          access: "read_only",
          source: { kind: "workspace", workspace_id: "w-1" },
        },
        {
          id: "ws:w-2",
          prefix: "/team-b/",
          access: "read_only",
          source: { kind: "workspace", workspace_id: "w-2" },
        },
      ],
    };
    const result = removeWorkspacePolicy(existingPolicy, "w-1");
    expect(result.rules).toHaveLength(2);
    expect(result.rules.find((r) => r.id === "manual-1")).toBeTruthy();
    expect(result.rules.find((r) => r.id === "ws:w-2")).toBeTruthy();
    expect(result.rules.find((r) => r.id === "ws:w-1")).toBeUndefined();
  });

  it("normalizeWorkspacePath rejects root and dot variants", () => {
    expect(() => normalizeWorkspacePath("/")).toThrow("must not be or resolve to '/'");
    expect(() => normalizeWorkspacePath(".")).toThrow("must not be or resolve to '/'");
    expect(() => normalizeWorkspacePath("..")).toThrow("must not be or resolve to '/'");
  });

  it("normalizeWorkspacePath decodes encoded path separators", () => {
    expect(normalizeWorkspacePath("team%2fagent")).toBe("/team/agent");
    expect(normalizeWorkspacePath("team%5cagent")).toBe("/team/agent");
    expect(normalizeWorkspacePath("x/%2e%2e/escape")).toBe("/escape");
    expect(normalizeWorkspacePath("%2e%2e/escape")).toBe("/escape");
  });

  it("normalizeWorkspacePath normalizes backslashes", () => {
    expect(normalizeWorkspacePath("team\\agent")).toBe("/team/agent");
  });

  it("preserves other workspaces when creating a new one", async () => {
    const created: WorkspaceRecord[] = [];
    (listWorkspaces as ReturnType<typeof vi.fn>).mockImplementation(async () => created);
    (createWorkspaceAtomic as ReturnType<typeof vi.fn>).mockImplementation(
      async (input: CreateWorkspaceAtomicInput) => {
        const now = new Date().toISOString();
        const record: WorkspaceRecord = {
          id: `generated-${input.name.toLowerCase().replace(/ /g, "-")}`,
          storageId: input.storageId,
          name: input.name,
          rootPath: input.rootPath,
          templateId: input.templateId,
          createdAt: now,
          updatedAt: now,
          memoryFiles: input.templateId === "coding"
            ? ["memory/tasks.md", "memory/decisions.md", "memory/handoff.md"]
            : [],
          checkpointIds: [],
        };
        created.push(record);
        return { workspace: record, policyUpdated: false, rollbackErrors: [] };
      },
    );

    await createAgentWorkspace({
      storageId: "local",
      name: "First",
      rootPath: "/first",
      templateId: "coding",
    });
    await createAgentWorkspace({
      storageId: "local",
      name: "Second",
      rootPath: "/second",
      templateId: "research",
    });
    const all = await listAgentWorkspaces();
    expect(all).toHaveLength(2);
  });

  it("migrates valid legacy workspaces independently and preserves failed or corrupt records", async () => {
    const validOne = {
      id: "legacy-1",
      storageId: "local",
      name: "Legacy one",
      rootPath: "/legacy-one",
      templateId: "coding",
      createdAt: "2026-01-01T00:00:00Z",
      updatedAt: "2026-01-01T00:00:00Z",
      memoryFiles: [],
      checkpointIds: [],
    };
    const validTwo = { ...validOne, id: "legacy-2", name: "Legacy two", rootPath: "/legacy-two" };
    const corrupt = { unexpected: true };
    window.localStorage.setItem(
      "infimount:agent-workspaces:v1",
      JSON.stringify([validOne, validTwo, corrupt]),
    );
    vi.mocked(apiImportLegacyWorkspaces)
      .mockResolvedValueOnce(1)
      .mockRejectedValueOnce(new Error("manifest mismatch"));

    const result = await migrateLegacyWorkspaces();

    expect(result.imported).toBe(1);
    expect(result.outcomes.map((outcome) => outcome.status)).toEqual([
      "imported",
      "failed",
      "invalid",
    ]);
    expect(JSON.parse(window.localStorage.getItem("infimount:agent-workspaces:v1")!)).toEqual([
      validTwo,
      corrupt,
    ]);
    expect(apiImportLegacyWorkspaces).toHaveBeenCalledTimes(2);
  });

  it("passes accessProfile to atomic command", async () => {
    const workspace = await createAgentWorkspace({
      storageId: "local",
      name: "Access-Test",
      rootPath: "/access-test",
      templateId: "coding",
      accessProfile: "read_only",
    });

    expect(workspace.id).toBeTruthy();
    expect(createWorkspaceAtomic).toHaveBeenCalledWith(
      expect.objectContaining({ accessProfile: "read_only" }),
    );
  });
});
