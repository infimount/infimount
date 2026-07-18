import { Label } from "@/components/ui/label";
import { Switch } from "@/components/ui/switch";
import { Button } from "@/components/ui/button";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Textarea } from "@/components/ui/textarea";
import type { McpStoragePolicy, StorageConfig } from "@/types/storage";

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

function splitPolicyPrefixes(value: string): string[] {
  const seen = new Set<string>();
  return value
    .split(/\r?\n/)
    .map(normalizePolicyPrefixInput)
    .filter(Boolean)
    .filter((item) => {
      if (seen.has(item)) {
        return false;
      }
      seen.add(item);
      return true;
    });
}

function normalizePolicyPrefixInput(value: string): string {
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

                <div className="grid gap-3 md:grid-cols-2">
                  <div className="space-y-2">
                    <Label className="text-xs font-normal text-muted-foreground">
                      Allowed prefixes
                    </Label>
                    <Textarea
                      value={policy.allowed_paths.join("\n")}
                      placeholder="Leave empty to allow all paths"
                      rows={3}
                      className={`border border-border/80 bg-card font-mono text-xs ${FIELD_FOCUS_CLASS}`}
                      onChange={(event) =>
                        onUpdatePolicyDraft(storage.id, (current) => ({
                          ...current,
                          allowed_paths: splitPolicyPrefixes(event.target.value),
                        }))
                      }
                    />
                  </div>
                  <div className="space-y-2">
                    <Label className="text-xs font-normal text-muted-foreground">
                      Denied prefixes
                    </Label>
                    <Textarea
                      value={policy.denied_paths.join("\n")}
                      placeholder="Example: private"
                      rows={3}
                      className={`border border-destructive/30 bg-destructive/5 font-mono text-xs ${FIELD_FOCUS_CLASS}`}
                      onChange={(event) =>
                        onUpdatePolicyDraft(storage.id, (current) => ({
                          ...current,
                          denied_paths: splitPolicyPrefixes(event.target.value),
                        }))
                      }
                    />
                  </div>
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
