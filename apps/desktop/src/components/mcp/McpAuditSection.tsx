import { ShieldAlert, Bell, RefreshCw, Check, X, Copy, Trash2 } from "lucide-react";

import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import type { McpNotificationPermission } from "@/lib/mcpNotifications";
import type {
  ActiveMcpSession,
  PendingMcpConfirmation,
  McpAuditEvent,
} from "@/types/storage";

const FIELD_FOCUS_CLASS =
  "focus-visible:border-ring focus-visible:ring-0 focus-visible:ring-offset-0";

function shortSessionId(sessionId: string): string {
  return sessionId.length > 12 ? `${sessionId.slice(0, 8)}…` : sessionId;
}

function describeSessionScope(session: ActiveMcpSession): string {
  const storages =
    session.allowed_storages.length > 0
      ? session.allowed_storages.join(", ")
      : "all exposed storages";
  const prefixes =
    session.allowed_prefixes.length > 0
      ? `prefixes: ${session.allowed_prefixes.join(", ")}`
      : "all paths";
  return `${storages}, ${prefixes}`;
}

interface McpAuditSectionProps {
  activeSessions: ActiveMcpSession[];
  pendingConfirmations: PendingMcpConfirmation[];
  auditEvents: McpAuditEvent[];
  notificationPermission: McpNotificationPermission;
  onRefreshAudit: () => Promise<void>;
  onClearAudit: () => Promise<void>;
  onEnableNotifications: () => Promise<void>;
  onApproveConfirmation: (operationId: string) => Promise<void>;
  onDenyConfirmation: (operationId: string) => Promise<void>;
  onCopyVisibleAudit: () => Promise<void>;
  onExportVisibleAudit: () => Promise<void>;
  filteredAuditEvents: McpAuditEvent[];
  auditQuery: string;
  onAuditQueryChange: (query: string) => void;
  auditDecisionFilter: string;
  onAuditDecisionFilterChange: (filter: string) => void;
  auditStorageFilter: string;
  onAuditStorageFilterChange: (filter: string) => void;
  auditDecisionOptions: string[];
  auditStorageOptions: string[];
}

export function McpAuditSection({
  activeSessions,
  pendingConfirmations,
  auditEvents,
  notificationPermission,
  onRefreshAudit,
  onClearAudit,
  onEnableNotifications,
  onApproveConfirmation,
  onDenyConfirmation,
  onCopyVisibleAudit,
  onExportVisibleAudit,
  filteredAuditEvents,
  auditQuery,
  onAuditQueryChange,
  auditDecisionFilter,
  onAuditDecisionFilterChange,
  auditStorageFilter,
  onAuditStorageFilterChange,
  auditDecisionOptions,
  auditStorageOptions,
}: McpAuditSectionProps) {
  return (
    <>
      <div className="space-y-3 rounded-xl border border-border/80 bg-card p-4 shadow-sm">
        <div className="flex flex-wrap items-start justify-between gap-3">
          <div>
            <div className="text-sm font-medium text-foreground">Active MCP Sessions</div>
            <p className="mt-1 text-xs leading-5 text-muted-foreground">
              Scoped sessions created by MCP clients. These are in-memory and expire locally.
            </p>
          </div>
          <Button
            type="button"
            variant="outline"
            size="sm"
            className="border-border/80"
            onClick={() => void onRefreshAudit()}
          >
            <RefreshCw className="mr-2 h-4 w-4" />
            Refresh
          </Button>
        </div>

        <div className="max-h-52 overflow-y-auto rounded-lg border border-border/80 bg-background">
          {activeSessions.length > 0 ? (
            activeSessions.map((session) => (
              <div
                key={session.id}
                className="grid gap-2 border-b border-border/70 px-3 py-3 text-xs last:border-b-0 md:grid-cols-[1fr_auto]"
              >
                <div className="min-w-0 space-y-1">
                  <div className="flex flex-wrap items-center gap-2">
                    <span className="font-mono text-foreground">{shortSessionId(session.id)}</span>
                    <span className="rounded-full border border-border bg-secondary/60 px-2 py-0.5 text-[11px] text-muted-foreground">
                      {session.read_only ? "read-only" : "read/write"}
                    </span>
                  </div>
                  <div className="truncate text-muted-foreground">
                    {describeSessionScope(session)}
                  </div>
                </div>
                <div className="text-muted-foreground md:text-right">
                  Expires {new Date(session.expires_at).toLocaleString()}
                </div>
              </div>
            ))
          ) : (
            <div className="px-3 py-6 text-center text-xs text-muted-foreground">
              No scoped MCP sessions are active.
            </div>
          )}
        </div>
      </div>

      <div className="space-y-3 rounded-xl border border-border/80 bg-card p-4 shadow-sm">
        <div className="flex flex-wrap items-start justify-between gap-3">
          <div className="flex items-start gap-3">
            <ShieldAlert className="mt-0.5 h-4 w-4 text-primary" />
            <div>
              <div className="text-sm font-medium text-foreground">
                Pending MCP Approvals
              </div>
              <p className="mt-1 text-xs leading-5 text-muted-foreground">
                Risky agent operations wait here. Notifications can point back to this queue,
                but approval happens in Infimount.
              </p>
            </div>
          </div>
          <div className="flex flex-wrap items-center gap-2">
            {notificationPermission !== "granted" &&
              notificationPermission !== "unsupported" && (
                <Button
                  type="button"
                  variant="outline"
                  size="sm"
                  className="border-border/80"
                  onClick={() => void onEnableNotifications()}
                >
                  <Bell className="mr-2 h-4 w-4" />
                  Enable notifications
                </Button>
              )}
            <Button
              type="button"
              variant="outline"
              size="sm"
              className="border-border/80"
              onClick={() => void onRefreshAudit()}
            >
              <RefreshCw className="mr-2 h-4 w-4" />
              Refresh
            </Button>
          </div>
        </div>

        <div className="max-h-64 overflow-y-auto rounded-lg border border-border/80 bg-background">
          {pendingConfirmations.length > 0 ? (
            pendingConfirmations.map((item) => (
              <div
                key={item.operation_id}
                className="grid gap-3 border-b border-border/70 px-3 py-3 text-xs last:border-b-0 md:grid-cols-[1fr_auto]"
              >
                <div className="min-w-0 space-y-1">
                  <div className="flex flex-wrap items-center gap-2">
                    <span className="font-mono text-foreground">{item.tool_name}</span>
                    <span className="rounded-full border border-amber-300/80 bg-amber-50 px-2 py-0.5 text-[11px] text-amber-800 dark:border-amber-700/60 dark:bg-amber-950/40 dark:text-amber-200">
                      {item.risk_type}
                    </span>
                    <span className="rounded-full border border-border bg-secondary/60 px-2 py-0.5 text-[11px] text-muted-foreground">
                      {item.operation}
                    </span>
                  </div>
                  <div className="truncate text-muted-foreground">{item.summary}</div>
                  <div className="truncate text-muted-foreground">
                    {item.storage_name} · {item.path}
                  </div>
                  <div className="text-[11px] text-muted-foreground">
                    Expires {new Date(item.expires_at).toLocaleString()}
                  </div>
                </div>
                <div className="flex items-center gap-2 md:justify-end">
                  <Button
                    type="button"
                    size="sm"
                    className="h-8 bg-primary text-primary-foreground hover:bg-primary/90"
                    onClick={() => void onApproveConfirmation(item.operation_id)}
                  >
                    <Check className="mr-2 h-4 w-4" />
                    Approve once
                  </Button>
                  <Button
                    type="button"
                    size="sm"
                    variant="outline"
                    className="h-8 border-border/80"
                    onClick={() => void onDenyConfirmation(item.operation_id)}
                  >
                    <X className="mr-2 h-4 w-4" />
                    Deny
                  </Button>
                </div>
              </div>
            ))
          ) : (
            <div className="px-3 py-6 text-center text-xs text-muted-foreground">
              No risky MCP operations are waiting for approval.
            </div>
          )}
        </div>
      </div>

      <div className="space-y-3 rounded-xl border border-border/80 bg-card p-4 shadow-sm">
        <div className="flex flex-wrap items-start justify-between gap-3">
          <div>
            <div className="text-sm font-medium text-foreground">MCP Audit</div>
            <p className="mt-1 text-xs leading-5 text-muted-foreground">
              Recent local MCP calls, denials, and failures. Secrets and presigned URL
              signatures are not stored here.
            </p>
          </div>
          <div className="flex flex-wrap gap-2">
            <Button
              type="button"
              variant="outline"
              size="sm"
              className="border-border/80"
              onClick={() => void onRefreshAudit()}
            >
              <RefreshCw className="mr-2 h-4 w-4" />
              Refresh
            </Button>
            <Button
              type="button"
              variant="outline"
              size="sm"
              className="border-border/80"
              onClick={() => void onCopyVisibleAudit()}
              disabled={filteredAuditEvents.length === 0}
            >
              <Copy className="mr-2 h-4 w-4" />
              Copy visible
            </Button>
            <Button
              type="button"
              variant="outline"
              size="sm"
              className="border-border/80"
              onClick={() => void onExportVisibleAudit()}
              disabled={filteredAuditEvents.length === 0}
            >
              Export visible
            </Button>
            <Button
              type="button"
              variant="outline"
              size="sm"
              className="border-border/80"
              onClick={() => void onClearAudit()}
              disabled={auditEvents.length === 0}
            >
              <Trash2 className="mr-2 h-4 w-4" />
              Clear
            </Button>
          </div>
        </div>
        <div className="grid gap-3 md:grid-cols-[1fr_160px_160px]">
          <Input
            value={auditQuery}
            onChange={(event) => onAuditQueryChange(event.target.value)}
            placeholder="Filter tool, path, error, or session"
            className={`border border-border bg-background text-xs ${FIELD_FOCUS_CLASS}`}
          />
          <Select value={auditDecisionFilter} onValueChange={onAuditDecisionFilterChange}>
            <SelectTrigger
              className={`h-9 border border-border bg-background text-xs ${FIELD_FOCUS_CLASS}`}
            >
              <SelectValue placeholder="Decision" />
            </SelectTrigger>
            <SelectContent className="border border-border bg-[hsl(var(--popover))] text-[hsl(var(--popover-foreground))] shadow-md">
              <SelectItem value="all">All decisions</SelectItem>
              {auditDecisionOptions.map((decision) => (
                <SelectItem key={decision} value={decision}>
                  {decision}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
          <Select value={auditStorageFilter} onValueChange={onAuditStorageFilterChange}>
            <SelectTrigger
              className={`h-9 border border-border bg-background text-xs ${FIELD_FOCUS_CLASS}`}
            >
              <SelectValue placeholder="Storage" />
            </SelectTrigger>
            <SelectContent className="border border-border bg-[hsl(var(--popover))] text-[hsl(var(--popover-foreground))] shadow-md">
              <SelectItem value="all">All storages</SelectItem>
              {auditStorageOptions.map((storageName) => (
                <SelectItem key={storageName} value={storageName}>
                  {storageName}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </div>
        <div className="text-[11px] text-muted-foreground">
          Showing {Math.min(filteredAuditEvents.length, 20)} of {filteredAuditEvents.length} matching events.
        </div>
        <div className="max-h-64 overflow-y-auto rounded-lg border border-border/80 bg-background">
          {filteredAuditEvents.length > 0 ? (
            filteredAuditEvents.slice(0, 20).map((event) => (
              <div
                key={event.id}
                className="grid gap-1 border-b border-border/70 px-3 py-2 text-xs last:border-b-0 md:grid-cols-[1fr_auto]"
              >
                <div className="min-w-0">
                  <div className="flex flex-wrap items-center gap-2">
                    <span className="font-mono text-foreground">{event.tool_name}</span>
                    <span className="rounded-full border border-border bg-secondary/50 px-2 py-0.5 text-[11px] text-muted-foreground">
                      {event.decision}
                    </span>
                    {event.matched_rule_id ? (
                      <span className="rounded-full border border-blue-300/60 bg-blue-50 px-2 py-0.5 text-[11px] text-blue-700 dark:border-blue-700/40 dark:bg-blue-950/30 dark:text-blue-300">
                        {event.matched_rule_id}
                      </span>
                    ) : null}
                    {event.workspace_id ? (
                      <span className="rounded-full border border-purple-300/60 bg-purple-50 px-2 py-0.5 text-[11px] text-purple-700 dark:border-purple-700/40 dark:bg-purple-950/30 dark:text-purple-300">
                        ws:{event.workspace_id}
                      </span>
                    ) : null}
                    {event.error_code ? (
                      <span className="text-[11px] text-destructive">
                        {event.error_code}
                      </span>
                    ) : null}
                  </div>
                  <div className="mt-1 truncate text-muted-foreground">
                    {event.storage_name ?? "-"} {event.path ? `· ${event.path}` : ""}
                  </div>
                </div>
                <div className="text-muted-foreground md:text-right">
                  {new Date(event.timestamp).toLocaleString()}
                </div>
              </div>
            ))
          ) : (
            <div className="px-3 py-6 text-center text-xs text-muted-foreground">
              {auditEvents.length === 0
                ? "No MCP audit events yet."
                : "No MCP audit events match these filters."}
            </div>
          )}
        </div>
      </div>
    </>
  );
}
