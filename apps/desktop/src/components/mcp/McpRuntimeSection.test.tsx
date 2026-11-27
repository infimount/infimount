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
  maxReadBytes: 2097152,
  maxWriteBytes: 4194304,
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

function renderSection(
  settings: McpSettings,
  runningHttp = false,
  options: {
    showNetworkWarning?: boolean;
    requiresHttpRestart?: boolean;
    nonLoopbackMissingAuth?: boolean;
  } = {},
) {
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
      nonLoopbackMissingAuth={options.nonLoopbackMissingAuth ?? false}
      showNetworkWarning={options.showNetworkWarning ?? false}
      requiresHttpRestart={options.requiresHttpRestart ?? false}
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
  it.each([
    { maxReadBytes: 0, maxWriteBytes: 1 },
    { maxReadBytes: 2097153, maxWriteBytes: 1 },
    { maxReadBytes: 1, maxWriteBytes: 4194305 },
    { maxReadBytes: 1, maxWriteBytes: 1.5 },
  ])("blocks saving invalid byte limits %o", (limits) => {
    const { onSave } = renderSection({ ...baseSettings, ...limits });
    const save = screen.getByRole("button", { name: "Save stdio Agent Access" });
    expect(save).toBeDisabled();
    expect(screen.getByRole("alert")).toHaveTextContent("positive whole bytes");
    fireEvent.click(save);
    expect(onSave).not.toHaveBeenCalled();
  });

  it("keeps stopping a running server available when draft byte limits are invalid", () => {
    const { onHttpToggle } = renderSection({ ...baseSettings, transport: "http", maxReadBytes: 0 }, true);
    const stop = screen.getByRole("button", { name: "Save & Start HTTP Server" });
    expect(stop).toBeEnabled();
    fireEvent.click(stop);
    expect(onHttpToggle).toHaveBeenCalledOnce();
  });

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

  it("lets users dismiss a persistent network-exposure warning without weakening start safety", () => {
    const httpSettings = {
      ...baseSettings,
      transport: "http" as const,
      bindAddress: "0.0.0.0",
    };
    renderSection(httpSettings, false, {
      showNetworkWarning: true,
      nonLoopbackMissingAuth: true,
    });

    expect(screen.getByText(/This bind address is not loopback/i)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Save & Start HTTP Server" })).toBeDisabled();
    fireEvent.click(screen.getByRole("button", { name: "Dismiss network exposure warning" }));
    expect(screen.queryByText(/This bind address is not loopback/i)).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Save & Start HTTP Server" })).toBeDisabled();
  });

  it("lets users dismiss the persistent HTTP restart warning", () => {
    const httpSettings = { ...baseSettings, transport: "http" as const };
    renderSection(httpSettings, true, { requiresHttpRestart: true });

    expect(screen.getByText(/Restart the HTTP server/i)).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Dismiss restart warning" }));
    expect(screen.queryByText(/Restart the HTTP server/i)).not.toBeInTheDocument();
  });
});
