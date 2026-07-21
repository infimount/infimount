import { useCallback, useRef, useState } from "react";
import { Upload, FileText, AlertTriangle, CheckCircle2, XCircle } from "lucide-react";

import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Button } from "@/components/ui/button";
import { Textarea } from "@/components/ui/textarea";
import { Label } from "@/components/ui/label";
import { Badge } from "@/components/ui/badge";
import { ScrollArea } from "@/components/ui/scroll-area";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import {
  previewStorageImport,
  applyStorageImport,
  type StorageImportPreview,
  type ApplyStorageImportRequest,
} from "@/lib/api";
import { useToast } from "@/hooks/use-toast";

interface StorageImportDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onImportComplete?: () => void;
}

type Step = "input" | "preview" | "applying";

export function StorageImportDialog({ open, onOpenChange, onImportComplete }: StorageImportDialogProps) {
  const { toast } = useToast();
  const fileRef = useRef<HTMLInputElement>(null);
  const [step, setStep] = useState<Step>("input");
  const [jsonInput, setJsonInput] = useState("");
  const [preview, setPreview] = useState<StorageImportPreview | null>(null);
  const [, setResult] = useState<{ applied: number; warnings: string[] } | null>(null);
  const [mode, setMode] = useState<"merge" | "replace">("merge");
  const [onConflict, setOnConflict] = useState<"error" | "overwrite" | "rename">("error");
  const [confirmed, setConfirmed] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const handleClose = useCallback(() => {
    onOpenChange(false);
    setTimeout(() => {
      setStep("input");
      setJsonInput("");
      setPreview(null);
      setResult(null);
      setError(null);
      setConfirmed(false);
    }, 200);
  }, [onOpenChange]);

  const handleFileUpload = useCallback(() => {
    fileRef.current?.click();
  }, []);

  const handleFileChange = useCallback((e: React.ChangeEvent<HTMLInputElement>) => {
    const file = e.target.files?.[0];
    if (!file) return;
    const reader = new FileReader();
    reader.onload = (ev) => {
      setJsonInput((ev.target?.result as string) ?? "");
    };
    reader.readAsText(file);
    e.target.value = "";
  }, []);

  const handlePreview = useCallback(async () => {
    if (!jsonInput.trim()) return;
    setError(null);
    try {
      const p = await previewStorageImport(jsonInput);
      setPreview(p);
      setStep("preview");
    } catch (err: unknown) {
      setError(err instanceof Error ? err.message : String(err));
    }
  }, [jsonInput]);

  const handleApply = useCallback(async () => {
    if (!preview) return;
    setStep("applying");
    setError(null);
    try {
      const request: ApplyStorageImportRequest = {
        previewId: preview.previewId,
        baseRegistryRevision: preview.baseRegistryRevision,
        mode,
        onConflict,
        confirmed,
      };
      const res = await applyStorageImport(request);
      setResult(res);
      toast({
        title: "Import successful",
        description: `Applied ${res.applied} storage configuration(s).${res.warnings.length ? " " + res.warnings.join(" ") : ""}`,
      });
      onImportComplete?.();
    } catch (err: unknown) {
      setError(err instanceof Error ? err.message : String(err));
      setStep("preview");
    }
  }, [preview, mode, onConflict, confirmed, toast, onImportComplete]);

  const changeCount = (preview?.additions.length ?? 0)
    + (preview?.updates.length ?? 0)
    + (preview?.removals.length ?? 0);

  const renderChangeList = (items: { name: string; backend: string; changeType: string }[], label: string, color: string) => {
    if (items.length === 0) return null;
    return (
      <div className="mb-2">
        <p className={`text-sm font-medium mb-1 text-${color}-600`}>{label} ({items.length})</p>
        <div className="space-y-1">
          {items.map((item, i) => (
            <div key={i} className="text-xs text-muted-foreground flex items-center gap-2">
              <Badge variant="outline" className="text-[10px] px-1">{item.backend}</Badge>
              <span>{item.name}</span>
              <span className="text-[10px] text-muted-foreground">— {item.changeType}</span>
            </div>
          ))}
        </div>
      </div>
    );
  };

  return (
    <Dialog open={open} onOpenChange={(o) => { if (!o) handleClose(); }}>
      <DialogContent className="sm:max-w-[600px]">
        <DialogHeader>
          <DialogTitle>Import Storage Configuration</DialogTitle>
          <DialogDescription>
            {step === "input" && "Paste a shareable export JSON or upload a file to preview and import."}
            {step === "preview" && `Review the ${changeCount} change(s) below before applying.`}
            {step === "applying" && "Applying import..."}
          </DialogDescription>
        </DialogHeader>

        {step === "input" && (
          <div className="space-y-4">
            <div className="flex gap-2">
              <Button variant="outline" size="sm" onClick={handleFileUpload}>
                <Upload className="h-4 w-4 mr-1" /> Upload File
              </Button>
              <input ref={fileRef} type="file" accept=".json" className="hidden" onChange={handleFileChange} />
            </div>
            <div>
              <Label htmlFor="import-json">JSON input</Label>
              <Textarea
                id="import-json"
                value={jsonInput}
                onChange={(e) => setJsonInput(e.target.value)}
                placeholder='Paste shareable export JSON here, e.g. {"schemaVersion":2,"kind":"infimount-shareable-config",...}'
                className="mt-1 font-mono text-xs min-h-[200px]"
              />
            </div>
            {error && (
              <div className="flex items-start gap-2 text-sm text-red-600 bg-red-50 p-3 rounded">
                <AlertTriangle className="h-4 w-4 mt-0.5 shrink-0" />
                <span>{error}</span>
              </div>
            )}
            <div className="flex justify-end gap-3 pt-2">
              <Button variant="outline" onClick={handleClose}>Cancel</Button>
              <Button onClick={handlePreview} disabled={!jsonInput.trim()}>
                <FileText className="h-4 w-4 mr-1" /> Preview Import
              </Button>
            </div>
          </div>
        )}

        {step === "preview" && preview && (
          <div className="space-y-4">
            <ScrollArea className="max-h-[300px] pr-4">
              {preview.additions.length === 0 && preview.updates.length === 0 && preview.removals.length === 0 && (
                <p className="text-sm text-muted-foreground">No changes detected.</p>
              )}
              {renderChangeList(preview.additions, "Additions", "green")}
              {renderChangeList(preview.updates, "Updates", "blue")}
              {renderChangeList(preview.removals, "Removals", "red")}
              {preview.warnings.map((w, i) => (
                <div key={i} className="flex items-start gap-2 text-xs text-amber-600 bg-amber-50 p-2 rounded mb-1">
                  <AlertTriangle className="h-3 w-3 mt-0.5 shrink-0" />
                  <span>{w}</span>
                </div>
              ))}
              {preview.missingSecretFields.length > 0 && (
                <div className="mt-2">
                  <p className="text-sm font-medium text-amber-600 mb-1">Missing secret fields</p>
                  {preview.missingSecretFields.map((f, i) => (
                    <div key={i} className="text-xs text-muted-foreground">{f.storageName}: {f.name}</div>
                  ))}
                </div>
              )}
            </ScrollArea>

            <div className="grid grid-cols-2 gap-3">
              <div>
                <Label htmlFor="import-mode">Mode</Label>
                <Select value={mode} onValueChange={(v: "merge" | "replace") => { setMode(v); setConfirmed(false); }}>
                  <SelectTrigger id="import-mode" className="mt-1">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value="merge">Merge</SelectItem>
                    <SelectItem value="replace">Replace</SelectItem>
                  </SelectContent>
                </Select>
              </div>
              <div>
                <Label htmlFor="import-conflict">On conflict</Label>
                <Select value={onConflict} onValueChange={(v: "error" | "overwrite" | "rename") => setOnConflict(v)}>
                  <SelectTrigger id="import-conflict" className="mt-1">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value="error">Error</SelectItem>
                    <SelectItem value="overwrite">Overwrite</SelectItem>
                    <SelectItem value="rename">Rename</SelectItem>
                  </SelectContent>
                </Select>
              </div>
            </div>

            {mode === "replace" && (
              <div className="flex items-start gap-2 text-sm text-amber-600 bg-amber-50 p-3 rounded">
                <AlertTriangle className="h-4 w-4 mt-0.5 shrink-0" />
                <div>
                  <p className="font-medium">Replace mode will remove all existing storages not in the import.</p>
                  <label className="flex items-center gap-2 mt-1 cursor-pointer">
                    <input type="checkbox" checked={confirmed} onChange={(e) => setConfirmed(e.target.checked)} className="rounded" />
                    <span className="text-xs">I understand, proceed with replace</span>
                  </label>
                </div>
              </div>
            )}

            {error && (
              <div className="flex items-start gap-2 text-sm text-red-600 bg-red-50 p-3 rounded">
                <XCircle className="h-4 w-4 mt-0.5 shrink-0" />
                <span>{error}</span>
              </div>
            )}

            <div className="flex justify-end gap-3 pt-2">
              <Button variant="outline" onClick={() => { setStep("input"); setPreview(null); setError(null); }}>
                Back
              </Button>
              <Button
                onClick={handleApply}
                disabled={mode === "replace" && !confirmed}
              >
                <CheckCircle2 className="h-4 w-4 mr-1" /> Apply Import
              </Button>
            </div>
          </div>
        )}

        {step === "applying" && (
          <div className="py-8 text-center text-muted-foreground">
            <div className="animate-spin h-6 w-6 border-2 border-primary border-t-transparent rounded-full mx-auto mb-2" />
            Applying storage configuration...
          </div>
        )}
      </DialogContent>
    </Dialog>
  );
}
