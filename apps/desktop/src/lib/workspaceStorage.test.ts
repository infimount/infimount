import { describe, expect, it } from "vitest";

import { workspaceStorageIssue } from "./workspaceStorage";
import type { StorageConfig } from "@/types/storage";

function localStorage(config: Record<string, unknown>): StorageConfig {
  return {
    id: "storage-1",
    name: "Local",
    backend: "local",
    type: "local-fs",
    config,
    enabled: true,
    mcpExposed: false,
    readOnly: false,
    connected: true,
    createdAt: "2026-09-11T00:00:00Z",
    updatedAt: "2026-09-11T00:00:00Z",
    mcpPolicy: {
      version: 2,
      default_access: "none",
      rules: [],
      denied_paths: [],
      confirmation_rules: {
        require_for_write: true,
        require_for_overwrite: true,
        require_for_delete: true,
        require_for_version_delete: true,
        require_for_presign: true,
        require_for_cross_storage_copy: true,
      },
    },
  };
}

describe("workspaceStorageIssue", () => {
  it("accepts legacy tilde roots supported by the local operator", () => {
    expect(workspaceStorageIssue(localStorage({ root: "~", rootPath: "~" }))).toBeNull();
    expect(workspaceStorageIssue(localStorage({ root: "~/projects" }))).toBeNull();
  });

  it("uses the same local-root alias precedence as the backend", () => {
    expect(
      workspaceStorageIssue(
        localStorage({ root: "relative/backend-root", rootPath: "/absolute/ui-root" }),
      ),
    ).toMatch(/not absolute/i);
  });

  it("rejects shell variables and arbitrary relative local roots", () => {
    expect(workspaceStorageIssue(localStorage({ root: "$HOME/projects" }))).toMatch(
      /shell variable/i,
    );
    expect(workspaceStorageIssue(localStorage({ root: "projects" }))).toMatch(/not absolute/i);
  });

  it("does not impose local filesystem path rules on other operators", () => {
    const storage = localStorage({ bucket: "workspace-bucket" });
    storage.backend = "s3";
    storage.type = "aws-s3";
    expect(workspaceStorageIssue(storage)).toBeNull();
  });
});
