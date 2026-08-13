import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { useState } from "react";
import { describe, expect, it, vi } from "vitest";
import { McpToolSection } from "./McpToolSection";
import type { McpSettings, McpStoragePolicy, McpToolDefinition, StorageConfig } from "@/types/storage";

const rules = {
  require_for_write: true,
  require_for_overwrite: false,
  require_for_delete: true,
  require_for_version_delete: false,
  require_for_presign: true,
  require_for_cross_storage_copy: false,
};
const policy: McpStoragePolicy = {
  version: 2, default_access: "read_write", rules: [], denied_paths: [], confirmation_rules: rules,
};
const storage = {
  id: "docs", type: "local-fs", name: "Documents", backend: "fs", config: {}, enabled: true,
  mcpExposed: true, readOnly: false, connected: true, createdAt: "2026-01-01", updatedAt: "2026-01-01", mcpPolicy: policy,
} as StorageConfig;
const tools: McpToolDefinition[] = [
  { name: "list_dir", description: "List files", category: "read", risk: "low", defaultEnabled: true },
  { name: "write_file", description: "Write files", category: "write", risk: "medium", defaultEnabled: false },
  { name: "delete_path", description: "Delete files", category: "destructive", risk: "high", defaultEnabled: false },
  { name: "generate_download_link", description: "Create links", category: "external_link", risk: "medium", defaultEnabled: false },
];
const initialSettings: McpSettings = {
  enabled: true, transport: "stdio", bindAddress: "127.0.0.1", port: 7433,
  enabledTools: ["list_dir"], securityBaselineVersion: 2, authTokenConfigured: true,
};

function renderSection(overrides: Partial<React.ComponentProps<typeof McpToolSection>> = {}) {
  const onSettingsChange = vi.fn();
  const onApplyPreset = vi.fn();
  const onCopy = vi.fn();
  const onTestServer = vi.fn(async () => undefined);
  function Harness() {
    const [settings, setSettings] = useState(initialSettings);
    return <McpToolSection
      tools={tools} settings={settings} onSettingsChange={(update) => {
        setSettings((current) => typeof update === "function" ? update(current) : update);
        onSettingsChange(update);
      }} isBusy={false} snippets={{ stdio: "stdio snippet", http: "http snippet" }} onCopy={onCopy}
      exposedStorages={[storage]} policyDrafts={{}} onApplyPreset={onApplyPreset} applyingPresetId={null}
      connectAssessment={{ label: "Safe", description: "Scoped", className: "text-green-600" }}
      accessCounts={{ readWrite: 1, readOnly: 0, noAccess: 0 }} readAccessSummary="all"
      writeAccessSummary="writes" destructiveAccessSummary="none" presignSummary="links"
      confirmationSummary="2 rules" showNetworkWarning activeSessions={[]} onTestServer={onTestServer}
      {...overrides}
    />;
  }
  return { ...render(<Harness />), onSettingsChange, onApplyPreset, onCopy, onTestServer };
}

describe("McpToolSection safety controls", () => {
  it("applies the safe preset, copies snippets, and tests the server", async () => {
    const { onSettingsChange, onCopy, onTestServer } = renderSection();
    fireEvent.click(screen.getByRole("button", { name: "Apply safe read-only" }));
    expect(onSettingsChange).toHaveBeenCalled();
    fireEvent.click(screen.getAllByRole("button", { name: "Copy" })[0]);
    expect(onCopy).toHaveBeenCalledWith("stdio snippet");
    fireEvent.click(screen.getByRole("button", { name: "Test" }));
    await waitFor(() => expect(onTestServer).toHaveBeenCalled());
    expect(screen.getByText("Network exposure")).toBeInTheDocument();
    expect(screen.getByText("Documents")).toBeInTheDocument();
  });

  it("requires confirmation for risky tools and advanced presets", async () => {
    const { onApplyPreset } = renderSection();
    fireEvent.click(screen.getByRole("button", { name: "Configure advanced tools" }));
    expect(screen.getByText("Advanced tools (disabled by default)")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("switch", { name: "Enable delete_path" }));
    expect(screen.getByRole("alertdialog")).toHaveTextContent("Enable delete_path?");
    fireEvent.click(screen.getByRole("button", { name: "Enable" }));
    fireEvent.click(screen.getByRole("button", { name: /Manual Approval/ }));
    expect(screen.getByRole("alertdialog")).toHaveTextContent("Apply Manual Approval?");
    fireEvent.click(screen.getByRole("button", { name: "Apply preset" }));
    await waitFor(() => expect(onApplyPreset).toHaveBeenCalled());
  });
});
