import { fireEvent, render, screen } from "@testing-library/react";
import { useState } from "react";
import { describe, expect, it, vi } from "vitest";
import { McpPolicySection } from "./McpPolicySection";
import type { McpStoragePolicy, StorageConfig } from "@/types/storage";

const policy: McpStoragePolicy = {
  version: 2, default_access: "read_write", rules: [], denied_paths: [],
  confirmation_rules: {
    require_for_write: true, require_for_overwrite: false, require_for_delete: true,
    require_for_version_delete: false, require_for_presign: true, require_for_cross_storage_copy: false,
  },
};
const storage = {
  id: "docs", type: "local-fs", name: "Documents", backend: "fs", config: {}, enabled: true,
  mcpExposed: true, readOnly: false, connected: true, createdAt: "2026-01-01", updatedAt: "2026-01-01", mcpPolicy: policy,
} as StorageConfig;

function renderPolicy(exposedStorages: StorageConfig[] = [storage]) {
  const onUpdatePolicyDraft = vi.fn();
  const onSavePolicy = vi.fn();
  function Harness() {
    const [draft, setDraft] = useState<Record<string, McpStoragePolicy>>({ docs: policy });
    return <McpPolicySection exposedStorages={exposedStorages} policyDrafts={draft}
      onUpdatePolicyDraft={(id, updater) => {
        onUpdatePolicyDraft(id, updater);
        setDraft((current) => ({ ...current, [id]: updater(current[id] ?? policy) }));
      }} onSavePolicy={onSavePolicy} savingPolicyId={null} />;
  }
  return { ...render(<Harness />), onUpdatePolicyDraft, onSavePolicy };
}

describe("McpPolicySection", () => {
  it("renders the empty state", () => {
    renderPolicy([]);
    expect(screen.getByText("No storage is exposed to MCP yet.")).toBeInTheDocument();
  });

  it("normalizes path rules, denied prefixes, confirmations, and saves", () => {
    const { onUpdatePolicyDraft, onSavePolicy } = renderPolicy();
    fireEvent.click(screen.getByRole("button", { name: "Add rule" }));
    const prefix = screen.getByPlaceholderText("Path prefix (e.g. projects)");
    fireEvent.change(prefix, { target: { value: " /projects\\./private/../data/ " } });
    fireEvent.blur(prefix);
    fireEvent.change(screen.getByPlaceholderText("Example: private"), {
      target: { value: "private\nprivate\\nested\n./public" },
    });
    fireEvent.click(screen.getAllByRole("switch")[0]);
    fireEvent.click(screen.getByRole("button", { name: "Save policy" }));
    expect(onUpdatePolicyDraft).toHaveBeenCalled();
    expect(onSavePolicy).toHaveBeenCalledWith("docs");
    expect(screen.getByPlaceholderText("Example: private")).toHaveValue("private\nprivate/nested\npublic");
  });
});
