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
  restoreWorkspaceMemoryCheckpoint,
} from "./agentWorkspaces";
import { createDirectory, readFile, writeFile } from "./api";
import type { McpStoragePolicy } from "@/types/storage";

vi.mock("./api", async (importOriginal) => {
  const actual = await importOriginal<typeof import("./api")>();
  return {
    ...actual,
    createDirectory: vi.fn(),
    listEntries: vi.fn(),
    readFile: vi.fn(),
    writeFile: vi.fn(),
  };
});

const policy: McpStoragePolicy = {
  default_access: "read_write",
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
      expect.objectContaining({ default_access: "none", allowed_paths: ["/agent space/"] }),
    );
    expect(listAgentWorkspaces()).toMatchObject([{ id: workspace.id, name: "Coding Workspace" }]);
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
    expect(buildWorkspacePolicy("/team/agent")).toMatchObject({
      default_access: "none",
      allowed_paths: ["/team/agent/"],
    });
  });
});
