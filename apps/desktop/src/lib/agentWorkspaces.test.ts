import { beforeEach, describe, expect, it, vi } from "vitest";

import {
  appendWorkspaceMemory,
  buildWorkspacePolicy,
  createAgentWorkspace,
  createWorkspaceCheckpoint,
  defaultWorkspacePath,
  listAgentWorkspaceCheckpoints,
  listAgentWorkspaces,
  normalizeWorkspacePath,
  removeWorkspacePolicy,
  restoreWorkspaceMemoryCheckpoint,
} from "./agentWorkspaces";
import {
  listWorkspaces,
  createWorkspace as apiCreateWorkspace,
  updateWorkspace as apiUpdateWorkspace,
  createDirectory,
  readFile,
  writeFile,
  type WorkspaceRecord,
} from "./api";
import type { McpStoragePolicy } from "@/types/storage";

vi.mock("./api", async (importOriginal) => {
  const actual = await importOriginal<typeof import("./api")>();
  return {
    ...actual,
    listWorkspaces: vi.fn().mockResolvedValue([]),
    createWorkspace: vi.fn().mockResolvedValue({}),
    updateWorkspace: vi.fn().mockResolvedValue({}),
    createDirectory: vi.fn().mockResolvedValue(undefined),
    listEntries: vi.fn().mockResolvedValue([]),
    readFile: vi.fn(),
    writeFile: vi.fn(),
  };
});

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

describe("agentWorkspaces", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    window.localStorage.clear();
    vi.mocked(createDirectory).mockResolvedValue(undefined);
    vi.mocked(writeFile).mockResolvedValue(undefined);
    vi.mocked(readFile).mockResolvedValue(new TextEncoder().encode("# Tasks\n"));
    (listWorkspaces as ReturnType<typeof vi.fn>).mockResolvedValue([]);
    (apiCreateWorkspace as ReturnType<typeof vi.fn>).mockResolvedValue({});
    (apiUpdateWorkspace as ReturnType<typeof vi.fn>).mockResolvedValue({});
  });

  it("creates a workspace, template files, and scoped MCP policy", async () => {
    const updatePolicy = vi.fn().mockResolvedValue(undefined);

    const workspace = await createAgentWorkspace({
      storageId: "local",
      name: "Coding Workspace",
      rootPath: "agent space",
      templateId: "coding",
      currentPolicy: policy,
      updatePolicy,
    });

    expect(workspace.rootPath).toBe("/agent space");
    expect(writeFile).toHaveBeenCalledWith(
      "local",
      "/agent space/README.md",
      expect.anything(),
    );
    expect(updatePolicy).toHaveBeenCalledWith(
      expect.objectContaining({ default_access: "read_write", rules: expect.any(Array) }),
    );
    expect(apiCreateWorkspace).toHaveBeenCalledWith(
      expect.objectContaining({ id: workspace.id, name: "Coding Workspace" }),
    );
  });

  it("appends memory and checkpoints memory files", async () => {
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
    expect(listAgentWorkspaceCheckpoints(workspace.id)).toMatchObject([{ id: checkpoint.id }]);
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

    window.localStorage.setItem("infimount:agent-workspace-checkpoints:v1", "[]");
    vi.mocked(readFile).mockResolvedValueOnce(manifestBytes);

    await restoreWorkspaceMemoryCheckpoint(workspace, checkpoint.id);

    expect(readFile).toHaveBeenCalledWith("local", `/research/${checkpoint.manifestPath}`);
    expect(writeFile).toHaveBeenCalledWith(
      "local",
      "/research/memory/questions.md",
      expect.anything(),
    );
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
    (apiCreateWorkspace as ReturnType<typeof vi.fn>).mockImplementation(
      async (input: WorkspaceRecord) => {
        const now = new Date().toISOString();
        const record: WorkspaceRecord = {
          ...input,
          createdAt: now,
          updatedAt: now,
          checkpointIds: input.checkpointIds ?? [],
        };
        created.push(record);
        return record;
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

  it("generates ID before policy call", async () => {
    const updatePolicy = vi.fn().mockResolvedValue(undefined);
    const workspace = await createAgentWorkspace({
      storageId: "local",
      name: "ID-First",
      rootPath: "/id-first",
      templateId: "coding",
      currentPolicy: policy,
      updatePolicy,
    });

    expect(workspace.id).toBeTruthy();
    const policyArg = updatePolicy.mock.calls[0][0] as McpStoragePolicy;
    const rule = policyArg.rules.find((candidate) => candidate.source.kind === "workspace");
    expect(rule).toBeDefined();
    expect(rule?.source.kind === "workspace" ? rule.source.workspace_id : undefined).toBe(
      workspace.id,
    );
  });
});
