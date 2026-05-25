export type ActivityLogEventType =
  | "transfer_planned"
  | "transfer_started"
  | "transfer_completed"
  | "transfer_failed"
  | "transfer_cancelled"
  | "transfer_recovery_started"
  | "workspace_created"
  | "workspace_memory_appended"
  | "workspace_checkpoint_created"
  | "workspace_checkpoint_restored";

export interface ActivityLogEvent {
  id: string;
  type: ActivityLogEventType;
  createdAt: number;
  jobId?: string;
  operation?: "copy" | "move" | "write" | "delete" | "mcp" | "workspace";
  sourceId?: string;
  targetId?: string;
  workspaceId?: string;
  pathCount?: number;
  summary?: Record<string, unknown>;
  message?: string;
}

const ACTIVITY_LOG_STORAGE_KEY = "infimount:activity-log:v1";
const MAX_ACTIVITY_EVENTS = 300;

function readEvents(): ActivityLogEvent[] {
  if (typeof window === "undefined") return [];
  try {
    const parsed = JSON.parse(window.localStorage.getItem(ACTIVITY_LOG_STORAGE_KEY) ?? "[]");
    if (!Array.isArray(parsed)) return [];
    return parsed.filter(
      (event): event is ActivityLogEvent =>
        Boolean(event) &&
        typeof event === "object" &&
        typeof event.id === "string" &&
        typeof event.type === "string" &&
        typeof event.createdAt === "number",
    );
  } catch {
    return [];
  }
}

export function listActivityLogEvents(): ActivityLogEvent[] {
  return readEvents().sort((a, b) => b.createdAt - a.createdAt);
}

export function appendActivityLogEvent(
  event: Omit<ActivityLogEvent, "id" | "createdAt"> & Partial<Pick<ActivityLogEvent, "id" | "createdAt">>,
): ActivityLogEvent {
  const nextEvent: ActivityLogEvent = {
    ...event,
    id: event.id ?? `activity-${Date.now()}-${Math.random().toString(36).slice(2, 9)}`,
    createdAt: event.createdAt ?? Date.now(),
  };

  if (typeof window !== "undefined") {
    const next = [nextEvent, ...readEvents()].slice(0, MAX_ACTIVITY_EVENTS);
    window.localStorage.setItem(ACTIVITY_LOG_STORAGE_KEY, JSON.stringify(next));
  }

  return nextEvent;
}

export function clearActivityLogEvents() {
  if (typeof window === "undefined") return;
  window.localStorage.removeItem(ACTIVITY_LOG_STORAGE_KEY);
}
