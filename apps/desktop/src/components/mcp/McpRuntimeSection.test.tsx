import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import { McpRuntimeSection } from "./McpRuntimeSection";
import type { McpRuntimeStatus, McpSettings } from "@/types/storage";

const baseSettings: McpSettings = {
  enabled: true,
  transport: "stdio",
  bindAddress: "127.0.0.1",
  port: 7331,
  enabledTools: ["list_dir"],
  securityBaselineVersion: 2,
  authTokenConfigured: false,
};

function status(settings: McpSettings, runningHttp = false): McpRuntimeStatus {
  return {
    settings,
    runningHttp,
    endpoint: runningHttp ? "http://127.0.0.1:7331/mcp" : null,
    endpointDisplay: runningHttp ? "http://127.0.0.1:7331/mcp" : "Starts on 127.0.0.1:7331/mcp",
    authTokenConfigured: false,
  };
}

function renderSection(settings: McpSettings, runningHttp = false) {
  const onSettingsChange = vi.fn();
  const onSave = vi.fn();
  const onHttpToggle = vi.fn();
  render(
    <McpRuntimeSection
      settings={settings}
      onSettingsChange={onSettingsChange}
      status={status(settings, runningHttp)}
      authTokenDraft={undefined}
      onAuthTokenDraftChange={vi.fn()}
      isBusy={false}
      isSaving={false}
      isTogglingHttp={false}
      nonLoopbackMissingAuth={false}
      showNetworkWarning={false}
      requiresHttpRestart={false}
      primaryActionLabel={settings.transport === "stdio" ? "Save stdio Agent Access" : "Save & Start HTTP Server"}
      endpointDisplay="Starts on 127.0.0.1:7331/mcp"
      onSave={onSave}
      onRotateAuthToken={vi.fn()}
      onHttpToggle={onHttpToggle}
    />,
  );
  return { onSettingsChange, onSave, onHttpToggle };
}

describe("McpRuntimeSection", () => {
  it("shows enabled stdio as on demand rather than stopped", () => {
    renderSection(baseSettings);
    expect(screen.getByText("On demand")).toBeInTheDocument();
    expect(screen.getByText(/Ready for client launch/i)).toBeInTheDocument();
    expect(screen.getByText("Client-launched stdio")).toBeInTheDocument();
    expect(screen.queryByText("HTTP server is not running.")).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /Start HTTP/i })).not.toBeInTheDocument();
  });

  it("lets advanced users explicitly disable the stdio Agent Access gate", () => {
    const { onSettingsChange } = renderSection(baseSettings);
    fireEvent.click(screen.getByRole("switch", { name: "Enable general Agent Access" }));
    expect(onSettingsChange).toHaveBeenCalledTimes(1);
  });

  it("keeps explicit start controls for HTTP", () => {
    const httpSettings = { ...baseSettings, transport: "http" as const };
    const { onHttpToggle } = renderSection(httpSettings);
    expect(screen.getByText("HTTP Runtime Status")).toBeInTheDocument();
    expect(screen.getByText("Stopped")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Save & Start HTTP Server" }));
    expect(onHttpToggle).toHaveBeenCalledTimes(1);
  });
});
