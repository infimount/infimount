import { Label } from "@/components/ui/label";
import { Switch } from "@/components/ui/switch";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Textarea } from "@/components/ui/textarea";
import { X, Plus } from "lucide-react";
import type { McpStoragePolicy, McpPathRule, StorageConfig } from "@/types/storage";

const FIELD_FOCUS_CLASS =
  "focus-visible:border-ring focus-visible:ring-0 focus-visible:ring-offset-0";

const CONFIRMATION_RULES: Array<{
  key: keyof McpStoragePolicy["confirmation_rules"];
  label: string;
}> = [
  { key: "require_for_write", label: "Confirm writes" },
  { key: "require_for_overwrite", label: "Confirm overwrites" },
  { key: "require_for_delete", label: "Confirm deletes" },
  { key: "require_for_version_delete", label: "Confirm version deletes" },
  { key: "require_for_presign", label: "Confirm download links" },
  { key: "require_for_cross_storage_copy", label: "Confirm cross-storage copy" },
];

function normalizePrefix(value: string): string {
  const segments: string[] = [];
  for (const segment of value.trim().replace(/\\/g, "/").split("/")) {
    if (!segment || segment === ".") {
      continue;
    }
    if (segment === "..") {
      segments.pop();
      continue;
    }
    segments.push(segment);
  }
  return segments.join("/");
}

function splitPolicyPrefixes(value: string): string[] {
  const seen = new Set<string>();
  return value
    .split(/\r?\n/)
    .map(normalizePrefix)
    .filter(Boolean)
    .filter((item) => {
      if (seen.has(item)) {
        return false;
      }
      seen.add(item);
      return true;
    });
}

function generateRuleId(): string {
  return `rule-${Date.now()}-${Math.random().toString(36).slice(2, 9)}`;
}

interface McpPolicySectionProps {
  exposedStorages: StorageConfig[];
  policyDrafts: Record<string, McpStoragePolicy>;
  onUpdatePolicyDraft: (
    storageId: string,
    updater: (policy: McpStoragePolicy) => McpStoragePolicy,
  ) => void;
  onSavePolicy: (storageId: string) => void;
  savingPolicyId: string | null;
}

export function McpPolicySection({
  exposedStorages,
  policyDrafts,
  onUpdatePolicyDraft,
  onSavePolicy,
  savingPolicyId,
}: McpPolicySectionProps) {
  return (
    <div className="space-y-3 rounded-xl border border-border/80 bg-card p-4 shadow-sm">
      <div>
        <div className="text-sm font-medium text-foreground">Path Policies</div>
        <p className="mt-1 text-xs leading-5 text-muted-foreground">
          Restrict exposed storages by access mode and path prefixes before any MCP tool
          reaches the backend.
        </p>
      </div>

      <div className="space-y-3">
        {exposedStorages.length > 0 ? (
          exposedStorages.map((storage) => {
            const policy = policyDrafts[storage.id] ?? storage.mcpPolicy;
            return (
              <div
                key={storage.id}
                className="space-y-3 rounded-lg border border-border/80 bg-background p-3"
              >
                <div className="flex flex-wrap items-center justify-between gap-3">
                  <div>
                    <div className="text-sm font-medium text-foreground">
                      {storage.name}
                    </div>
                    <div className="text-xs text-muted-foreground">{storage.backend}</div>
                  </div>
                  <div className="w-44">
                    <Select
                      value={policy.default_access}
                      onValueChange={(value) =>
                        onUpdatePolicyDraft(storage.id, (current) => ({
                          ...current,
                          default_access: value as McpStoragePolicy["default_access"],
                        }))
                      }
                    >
                      <SelectTrigger
                        className={`h-9 border border-border bg-card text-xs text-[hsl(var(--card-foreground))] ${FIELD_FOCUS_CLASS}`}
                      >
                        <SelectValue />
                      </SelectTrigger>
                      <SelectContent className="border border-border bg-[hsl(var(--popover))] text-[hsl(var(--popover-foreground))] shadow-md">
                        <SelectItem value="none">No access</SelectItem>
                        <SelectItem value="read_only">Read only</SelectItem>
                        <SelectItem value="read_write">Read / write</SelectItem>
                      </SelectContent>
                    </Select>
                  </div>
                </div>

                <div className="space-y-2">
                  <div className="flex items-center justify-between">
                    <Label className="text-xs font-normal text-muted-foreground">
                      Rules
                    </Label>
                    <Button
                      type="button"
                      size="sm"
                      variant="outline"
                      className="h-7 gap-1 border-dashed text-xs"
                      onClick={() =>
                        onUpdatePolicyDraft(storage.id, (current) => ({
                          ...current,
                          rules: [
                            ...current.rules,
                            {
                              id: generateRuleId(),
                              prefix: "",
                              access: "read_write" as McpStoragePolicy["default_access"],
                              source: { kind: "manual" },
                            },
                          ],
                        }))
                      }
                    >
                      <Plus className="h-3 w-3" />
                      Add rule
                    </Button>
                  </div>
                  {policy.rules.length > 0 ? (
                    <div className="space-y-2">
                      {policy.rules.map((rule, index) => (
                        <div
                          key={rule.id}
                          className="flex items-start gap-2 rounded-md border border-border/70 bg-card p-2"
                        >
                          <div className="min-w-0 flex-1">
                            <Input
                              value={rule.prefix}
                              placeholder="Path prefix (e.g. projects)"
                              className={`h-8 border-border/80 font-mono text-xs ${FIELD_FOCUS_CLASS}`}
                              onChange={(event) =>
                                onUpdatePolicyDraft(storage.id, (current) => {
                                  const next = [...current.rules];
                                  next[index] = {
                                    ...next[index],
                                    prefix: event.target.value,
                                  };
                                  return { ...current, rules: next };
                                })
                              }
                              onBlur={(event) =>
                                onUpdatePolicyDraft(storage.id, (current) => {
                                  const next = [...current.rules];
                                  next[index] = {
                                    ...next[index],
                                    prefix: normalizePrefix(event.target.value),
                                  };
                                  return { ...current, rules: next };
                                })
                              }
                            />
                          </div>
                          <Select
                            value={rule.access}
                            onValueChange={(value) =>
                              onUpdatePolicyDraft(storage.id, (current) => {
                                const next = [...current.rules];
                                next[index] = {
                                  ...next[index],
                                  access: value as McpPathRule["access"],
                                };
                                return { ...current, rules: next };
                              })
                            }
                          >
                            <SelectTrigger
                              className={`h-8 w-28 border border-border bg-card text-xs ${FIELD_FOCUS_CLASS}`}
                            >
                              <SelectValue />
                            </SelectTrigger>
                            <SelectContent className="border border-border bg-[hsl(var(--popover))] text-[hsl(var(--popover-foreground))] shadow-md">
                              <SelectItem value="read_only">Read only</SelectItem>
                              <SelectItem value="read_write">Read / write</SelectItem>
                            </SelectContent>
                          </Select>
                          {rule.source.kind === "manual" ? (
                            <Button
                              type="button"
                              size="sm"
                              variant="ghost"
                              className="h-8 w-8 p-0 text-muted-foreground hover:text-destructive"
                              onClick={() =>
                                onUpdatePolicyDraft(storage.id, (current) => ({
                                  ...current,
                                  rules: current.rules.filter((r) => r.id !== rule.id),
                                }))
                              }
                            >
                              <X className="h-4 w-4" />
                            </Button>
                          ) : (
                            <div className="flex h-8 w-8 items-center justify-center" title="Managed by workspace">
                              <span className="text-[10px] text-muted-foreground/50">WS</span>
                            </div>
                          )}
                        </div>
                      ))}
                    </div>
                  ) : (
                    <div className="rounded-md border border-dashed border-border/50 px-3 py-4 text-center text-[11px] text-muted-foreground">
                      No path rules. Default access applies to all paths.
                    </div>
                  )}
                </div>

                <div className="space-y-2">
                  <Label className="text-xs font-normal text-muted-foreground">
                    Denied prefixes
                  </Label>
                  <Textarea
                    value={policy.denied_paths.join("\n")}
                    placeholder="Example: private"
                    rows={2}
                    className={`border border-destructive/30 bg-destructive/5 font-mono text-xs ${FIELD_FOCUS_CLASS}`}
                    onChange={(event) =>
                      onUpdatePolicyDraft(storage.id, (current) => ({
                        ...current,
                        denied_paths: splitPolicyPrefixes(event.target.value),
                      }))
                    }
                  />
                </div>

                <div className="grid gap-2 md:grid-cols-2">
                  {CONFIRMATION_RULES.map((rule) => (
                    <div
                      key={rule.key}
                      className="flex items-center justify-between gap-3 rounded-md border border-border/70 bg-card px-3 py-2"
                    >
                      <span className="text-xs text-muted-foreground">{rule.label}</span>
                      <Switch
                        checked={policy.confirmation_rules[rule.key]}
                        onCheckedChange={(value) =>
                          onUpdatePolicyDraft(storage.id, (current) => ({
                            ...current,
                            confirmation_rules: {
                              ...current.confirmation_rules,
                              [rule.key]: value,
                            },
                          }))
                        }
                      />
                    </div>
                  ))}
                </div>

                <div className="flex justify-end">
                  <Button
                    type="button"
                    size="sm"
                    className="h-8 bg-primary text-primary-foreground hover:bg-primary/90"
                    onClick={() => void onSavePolicy(storage.id)}
                    disabled={savingPolicyId === storage.id}
                  >
                    {savingPolicyId === storage.id ? "Saving..." : "Save policy"}
                  </Button>
                </div>
              </div>
            );
          })
        ) : (
          <div className="rounded-lg border border-border/80 bg-background px-3 py-6 text-center text-xs text-muted-foreground">
            No storage is exposed to MCP yet.
          </div>
        )}
      </div>
    </div>
  );
}
