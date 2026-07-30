import { Bug, CheckCircle2, Download, ExternalLink, FileText, ShieldCheck, XCircle } from "lucide-react";
import { useCallback, useState } from "react";

import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Separator } from "@/components/ui/separator";
import { toast } from "@/hooks/use-toast";
import {
  exportDiagnostics as apiExportDiagnostics,
  getOsInfo,
  revealDiagnosticsExport,
} from "@/lib/api";
import type { DiagnosticsExportResult, OsInfo } from "@/types/diagnostics";

interface DiagnosticsDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
}

export function DiagnosticsDialog({ open, onOpenChange }: DiagnosticsDialogProps) {
  const [exportResult, setExportResult] = useState<DiagnosticsExportResult | null>(null);
  const [osInfo, setOsInfo] = useState<OsInfo | null>(null);
  const [isExporting, setIsExporting] = useState(false);
  const [isLoadingInfo, setIsLoadingInfo] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const loadInfo = useCallback(async () => {
    setIsLoadingInfo(true);
    try {
      const info = await getOsInfo();
      setOsInfo(info);
    } catch {
      // ignore
    }
    setIsLoadingInfo(false);
  }, []);

  const handleExport = async () => {
    setIsExporting(true);
    setError(null);
    try {
      const result = await apiExportDiagnostics();
      setExportResult(result);
      toast({
        title: "Diagnostics exported",
        description: `Created ${result.bundleName}`,
      });
    } catch (e) {
      setError(e instanceof Error ? e.message : "Export failed");
    }
    setIsExporting(false);
  };

  const handleOpenFolder = async () => {
    if (!exportResult) return;
    try {
      await revealDiagnosticsExport(exportResult.exportId);
    } catch {
      toast({
        title: "Diagnostics path",
        description: exportResult.bundleName,
      });
    }
  };

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-[600px]">
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2">
            <Bug className="h-5 w-5 text-primary" />
            Diagnostics
          </DialogTitle>
          <DialogDescription>
            Export a privacy-safe diagnostics bundle for troubleshooting.
          </DialogDescription>
        </DialogHeader>

        <div className="space-y-4">
          {osInfo && (
            <div className="rounded-xl border border-border/80 bg-card p-3">
              <div className="text-sm font-medium">System info</div>
              <div className="mt-2 space-y-1 text-xs text-muted-foreground">
                <div className="flex justify-between">
                  <span>App version</span>
                  <span className="font-mono">{osInfo.appVersion}</span>
                </div>
                <div className="flex justify-between">
                  <span>OS / Arch</span>
                  <span className="font-mono">{osInfo.osArch}</span>
                </div>
              </div>
            </div>
          )}

          <div className="rounded-xl border border-amber-500/20 bg-amber-50/50 p-3 dark:bg-amber-950/10">
            <div className="flex items-start gap-2">
              <ShieldCheck className="mt-0.5 h-4 w-4 shrink-0 text-amber-600" />
              <div className="text-xs leading-5 text-amber-700 dark:text-amber-400">
                <strong>Privacy-safe.</strong> The diagnostics bundle contains app and sidecar
                status, storage/backend counts, sanitized errors, and up to 100 schema-limited
                product-event and MCP audit summaries. It excludes storage names, file paths,
                storage endpoints, credentials, tokens, and contents, then validates the generated
                files against a local sensitive-value corpus. Review the manifest before sharing.
              </div>
            </div>
          </div>

          {exportResult && (
            <div className="rounded-xl border border-green-200 bg-green-50 p-3 dark:border-green-900/30 dark:bg-green-950/10">
              <div className="flex items-center gap-2">
                <CheckCircle2 className="h-4 w-4 text-green-600" />
                <span className="text-sm font-medium text-green-700 dark:text-green-400">
                  Exported
                </span>
              </div>
              <p className="mt-1 text-xs text-green-600 dark:text-green-400 break-all">
                {exportResult.bundleName}
              </p>
              <div className="mt-2 flex flex-wrap gap-2">
                <Button
                  type="button"
                  variant="outline"
                  size="sm"
                  onClick={handleOpenFolder}
                >
                  <ExternalLink className="mr-1 h-3 w-3" />
                  Open folder
                </Button>
              </div>
            </div>
          )}

          {error && (
            <div className="flex items-center gap-2 rounded-xl border border-red-200 bg-red-50 p-3 dark:border-red-900/30 dark:bg-red-950/10">
              <XCircle className="h-4 w-4 shrink-0 text-red-600" />
              <span className="text-sm text-red-700 dark:text-red-400">{error}</span>
            </div>
          )}
        </div>

        <Separator />

        <div className="flex justify-between">
          <Button type="button" variant="ghost" onClick={loadInfo} disabled={isLoadingInfo}>
            <FileText className="mr-1 h-4 w-4" />
            {isLoadingInfo ? "Loading..." : "Refresh info"}
          </Button>
          <Button type="button" onClick={handleExport} disabled={isExporting}>
            <Download className="mr-1 h-4 w-4" />
            {isExporting ? "Exporting..." : "Export diagnostics"}
          </Button>
        </div>
      </DialogContent>
    </Dialog>
  );
}
