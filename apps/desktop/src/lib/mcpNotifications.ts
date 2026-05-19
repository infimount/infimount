import type { PendingMcpConfirmation } from "@/types/storage";

export type McpNotificationPermission = NotificationPermission | "unsupported";

export function getMcpNotificationPermission(): McpNotificationPermission {
  if (!("Notification" in globalThis)) {
    return "unsupported";
  }
  return Notification.permission;
}

export async function requestMcpNotificationPermission(): Promise<McpNotificationPermission> {
  if (!("Notification" in globalThis)) {
    return "unsupported";
  }
  return Notification.requestPermission();
}

export function notifyPendingMcpConfirmation(
  pending: PendingMcpConfirmation,
  onClick?: () => void,
): boolean {
  if (!("Notification" in globalThis) || Notification.permission !== "granted") {
    return false;
  }

  const notification = new Notification("Infimount MCP approval needed", {
    body: `${pending.tool_name} wants ${pending.risk_type} access on ${pending.storage_name}.`,
    tag: `infimount-mcp-${pending.operation_id}`,
    silent: false,
  });

  notification.onclick = () => {
    window.focus();
    onClick?.();
    notification.close();
  };

  return true;
}
