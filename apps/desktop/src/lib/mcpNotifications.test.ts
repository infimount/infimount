import { describe, expect, it, vi } from "vitest";
import {
  getMcpNotificationPermission,
  notifyPendingMcpConfirmation,
  requestMcpNotificationPermission,
} from "./mcpNotifications";
import type { PendingMcpConfirmation } from "@/types/storage";

const pending: PendingMcpConfirmation = {
  operation_id: "op-1",
  tool_name: "delete_path",
  operation: "delete",
  risk_type: "delete",
  storage_id: "storage-1",
  storage_name: "Local",
  path: "/Local/report.txt",
  summary: "delete_path on /Local/report.txt",
  created_at: "2026-01-01T00:00:00Z",
  expires_at: "2026-01-01T00:05:00Z",
};

describe("mcpNotifications", () => {
  it("reports unsupported permission state when notifications are unavailable", async () => {
    const original = globalThis.Notification;
    Reflect.deleteProperty(globalThis, "Notification");

    expect(getMcpNotificationPermission()).toBe("unsupported");
    await expect(requestMcpNotificationPermission()).resolves.toBe("unsupported");
    expect(notifyPendingMcpConfirmation(pending)).toBe(false);

    if (original) {
      vi.stubGlobal("Notification", original);
    }
  });

  it("reads and requests notification permission", async () => {
    const requestPermission = vi.fn().mockResolvedValue("granted");
    const notification = vi.fn();
    Object.assign(notification, {
      permission: "default",
      requestPermission,
    });
    vi.stubGlobal("Notification", notification);

    expect(getMcpNotificationPermission()).toBe("default");
    await expect(requestMcpNotificationPermission()).resolves.toBe("granted");
    expect(requestPermission).toHaveBeenCalled();

    vi.unstubAllGlobals();
  });

  it("does not notify when permission is not granted", () => {
    const notification = vi.fn();
    Object.assign(notification, { permission: "denied" });
    vi.stubGlobal("Notification", notification);

    expect(notifyPendingMcpConfirmation(pending)).toBe(false);
    expect(notification).not.toHaveBeenCalled();

    vi.unstubAllGlobals();
  });

  it("shows a desktop notification for pending MCP approval without leaking file contents", () => {
    const close = vi.fn();
    const notification = vi.fn(function MockNotification() {
      return { close, onclick: null };
    });
    Object.assign(notification, { permission: "granted" });
    vi.stubGlobal("Notification", notification);

    expect(notifyPendingMcpConfirmation(pending)).toBe(true);
    expect(notification).toHaveBeenCalledWith(
      "Infimount MCP approval needed",
      expect.objectContaining({
        body: "delete_path wants delete access on Local.",
        tag: "infimount-mcp-op-1",
      }),
    );

    vi.unstubAllGlobals();
  });

  it("focuses the window, runs the click handler, and closes the notification on click", () => {
    const close = vi.fn();
    const notification = vi.fn(function MockNotification() {
      return { close, onclick: null as (() => void) | null };
    });
    Object.assign(notification, { permission: "granted" });
    vi.stubGlobal("Notification", notification);
    const focus = vi.spyOn(window, "focus").mockImplementation(() => undefined);
    const onClick = vi.fn();

    expect(notifyPendingMcpConfirmation(pending, onClick)).toBe(true);
    const createdNotification = notification.mock.results[0].value as { onclick: (() => void) | null };
    createdNotification.onclick?.();

    expect(focus).toHaveBeenCalled();
    expect(onClick).toHaveBeenCalled();
    expect(close).toHaveBeenCalled();

    focus.mockRestore();
    vi.unstubAllGlobals();
  });
});
