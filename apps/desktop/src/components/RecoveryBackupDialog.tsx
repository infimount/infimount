import { useCallback, useRef, useState } from "react";
import { Download, Upload, Shield, AlertTriangle, CheckCircle2, Eye, EyeOff } from "lucide-react";

import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { Switch } from "@/components/ui/switch";
import {
  createRecoveryBackup,
  previewRecoveryRestore,
  applyRecoveryRestore,
  type CreateBackupResult,
  type RestorePreviewResult,
} from "@/lib/api";
import { useToast } from "@/hooks/use-toast";

interface RecoveryBackupDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onRestoreComplete?: () => void;
}

export function RecoveryBackupDialog({ open, onOpenChange, onRestoreComplete }: RecoveryBackupDialogProps) {
  const { toast } = useToast();
  const fileRef = useRef<HTMLInputElement>(null);

  const [activeTab, setActiveTab] = useState("create");

  const [createPassphrase, setCreatePassphrase] = useState("");
  const [createConfirm, setCreateConfirm] = useState("");
  const [showCreatePass, setShowCreatePass] = useState(false);
  const [creating, setCreating] = useState(false);
  const [createResult, setCreateResult] = useState<CreateBackupResult | null>(null);

  const [restorePassphrase, setRestorePassphrase] = useState("");
  const [showRestorePass, setShowRestorePass] = useState(false);
  const [restoreArmored, setRestoreArmored] = useState("");
  const [restorePreview, setRestorePreview] = useState<RestorePreviewResult | null>(null);
  const [restoreMcp, setRestoreMcp] = useState(true);
  const [restoreApp, setRestoreApp] = useState(true);
  const [restoring, setRestoring] = useState(false);
  const [restoreError, setRestoreError] = useState<string | null>(null);

  const handleClose = useCallback(() => {
    onOpenChange(false);
    setTimeout(() => {
      setActiveTab("create");
      setCreatePassphrase("");
      setCreateConfirm("");
      setCreating(false);
      setCreateResult(null);
      setRestorePassphrase("");
      setRestoreArmored("");
      setRestorePreview(null);
      setRestoring(false);
      setRestoreError(null);
    }, 200);
  }, [onOpenChange]);

  const handleCreate = useCallback(async () => {
    if (createPassphrase.length < 8) {
      toast({ title: "Passphrase too short", description: "Use at least 8 characters.", variant: "destructive" });
      return;
    }
    if (createPassphrase !== createConfirm) {
      toast({ title: "Passphrases don't match", variant: "destructive" });
      return;
    }
    setCreating(true);
    try {
      const result = await createRecoveryBackup({ passphrase: createPassphrase });
      setCreateResult(result);
      toast({
        title: "Backup created",
        description: `Encrypted backup with ${result.storageCount} storage(s).`,
      });
    } catch (err: unknown) {
      toast({
        title: "Backup failed",
        description: err instanceof Error ? err.message : String(err),
        variant: "destructive",
      });
    } finally {
      setCreating(false);
    }
  }, [createPassphrase, createConfirm, toast]);

  const handleDownload = useCallback(() => {
    if (!createResult) return;
    const blob = new Blob([createResult.armored], { type: "text/plain" });
    const url = URL.createObjectURL(blob);
    const link = document.createElement("a");
    link.href = url;
    link.download = `infimount-recovery-backup-${Date.now()}.age`;
    link.click();
    URL.revokeObjectURL(url);
  }, [createResult]);

  const handleFileUpload = useCallback(() => {
    fileRef.current?.click();
  }, []);

  const handleFileChange = useCallback((e: React.ChangeEvent<HTMLInputElement>) => {
    const file = e.target.files?.[0];
    if (!file) return;
    const reader = new FileReader();
    reader.onload = (ev) => {
      setRestoreArmored((ev.target?.result as string) ?? "");
    };
    reader.readAsText(file);
    e.target.value = "";
  }, []);

  const handlePreviewRestore = useCallback(async () => {
    if (!restoreArmored.trim() || !restorePassphrase) return;
    setRestoreError(null);
    setRestorePreview(null);
    try {
      const preview = await previewRecoveryRestore({ passphrase: restorePassphrase, armored: restoreArmored });
      if (!preview.checksumValid) {
        setRestoreError("Backup checksum mismatch; data may be corrupted.");
        return;
      }
      setRestorePreview(preview);
    } catch (err: unknown) {
      setRestoreError(err instanceof Error ? err.message : String(err));
    }
  }, [restoreArmored, restorePassphrase]);

  const handleApplyRestore = useCallback(async () => {
    if (!restorePreview) return;
    setRestoring(true);
    setRestoreError(null);
    try {
      const result = await applyRecoveryRestore({
        passphrase: restorePassphrase,
        armored: restoreArmored,
        restoreMcpSettings: restoreMcp,
        restoreAppSettings: restoreApp,
      });
      toast({
        title: "Restore complete",
        description: `Restored ${result.storagesRestored} storage(s), MCP settings: ${result.mcpSettingsRestored}, app settings: ${result.appSettingsRestored}.`,
      });
      onRestoreComplete?.();
      handleClose();
    } catch (err: unknown) {
      setRestoreError(err instanceof Error ? err.message : String(err));
    } finally {
      setRestoring(false);
    }
  }, [restorePreview, restorePassphrase, restoreArmored, restoreMcp, restoreApp, toast, onRestoreComplete, handleClose]);

  return (
    <Dialog open={open} onOpenChange={(o) => { if (!o) handleClose(); }}>
      <DialogContent className="sm:max-w-[540px]">
        <DialogHeader>
          <DialogTitle>Recovery Backup</DialogTitle>
          <DialogDescription>
            Create an encrypted backup of all storage configurations or restore from one.
          </DialogDescription>
        </DialogHeader>

        <Tabs value={activeTab} onValueChange={setActiveTab}>
          <TabsList className="grid grid-cols-2">
            <TabsTrigger value="create"><Download className="h-4 w-4 mr-1" /> Create Backup</TabsTrigger>
            <TabsTrigger value="restore"><Upload className="h-4 w-4 mr-1" /> Restore</TabsTrigger>
          </TabsList>

          <TabsContent value="create" className="space-y-4 pt-4">
            {!createResult ? (
              <>
                <div>
                  <Label htmlFor="backup-pass">Encryption passphrase</Label>
                  <div className="relative mt-1">
                    <Input
                      id="backup-pass"
                      type={showCreatePass ? "text" : "password"}
                      value={createPassphrase}
                      onChange={(e) => setCreatePassphrase(e.target.value)}
                      placeholder="Enter a strong passphrase (min 8 chars)"
                    />
                    <button
                      type="button"
                      onClick={() => setShowCreatePass(!showCreatePass)}
                      className="absolute right-2 top-1/2 -translate-y-1/2 text-muted-foreground"
                    >
                      {showCreatePass ? <EyeOff className="h-4 w-4" /> : <Eye className="h-4 w-4" />}
                    </button>
                  </div>
                </div>
                <div>
                  <Label htmlFor="backup-confirm">Confirm passphrase</Label>
                  <Input
                    id="backup-confirm"
                    type="password"
                    value={createConfirm}
                    onChange={(e) => setCreateConfirm(e.target.value)}
                    placeholder="Re-enter passphrase"
                    className="mt-1"
                  />
                </div>
                <div className="flex justify-end gap-3 pt-2">
                  <Button variant="outline" onClick={handleClose}>Cancel</Button>
                  <Button onClick={handleCreate} disabled={creating}>
                    {creating ? "Encrypting..." : "Create Backup"}
                  </Button>
                </div>
              </>
            ) : (
              <div className="space-y-4">
                <div className="flex items-start gap-3 bg-green-50 p-4 rounded">
                  <CheckCircle2 className="h-5 w-5 text-green-600 mt-0.5 shrink-0" />
                  <div>
                    <p className="font-medium text-green-800">Backup created successfully</p>
                    <p className="text-sm text-green-700 mt-1">
                      {createResult.storageCount} storage configuration(s) encrypted.
                    </p>
                  </div>
                </div>
                <div className="flex justify-end gap-3 pt-2">
                  <Button variant="outline" onClick={handleClose}>Close</Button>
                  <Button onClick={handleDownload}>
                    <Download className="h-4 w-4 mr-1" /> Download Backup
                  </Button>
                </div>
              </div>
            )}
          </TabsContent>

          <TabsContent value="restore" className="space-y-4 pt-4">
            <div className="flex gap-2">
              <Button variant="outline" size="sm" onClick={handleFileUpload}>
                <Upload className="h-4 w-4 mr-1" /> Upload Backup File
              </Button>
              <input ref={fileRef} type="file" accept=".age,.txt" className="hidden" onChange={handleFileChange} />
            </div>

            <div>
              <Label htmlFor="restore-armored">Armored backup content</Label>
              <textarea
                id="restore-armored"
                value={restoreArmored}
                onChange={(e) => setRestoreArmored(e.target.value)}
                placeholder="Paste the armored .age backup content here"
                className="mt-1 w-full min-h-[120px] rounded border border-input bg-background px-3 py-2 text-xs font-mono"
              />
            </div>

            <div>
              <Label htmlFor="restore-pass">Decryption passphrase</Label>
              <div className="relative mt-1">
                <Input
                  id="restore-pass"
                  type={showRestorePass ? "text" : "password"}
                  value={restorePassphrase}
                  onChange={(e) => setRestorePassphrase(e.target.value)}
                  placeholder="Enter the backup passphrase"
                />
                <button
                  type="button"
                  onClick={() => setShowRestorePass(!showRestorePass)}
                  className="absolute right-2 top-1/2 -translate-y-1/2 text-muted-foreground"
                >
                  {showRestorePass ? <EyeOff className="h-4 w-4" /> : <Eye className="h-4 w-4" />}
                </button>
              </div>
            </div>

            {restoreError && (
              <div className="flex items-start gap-2 text-sm text-red-600 bg-red-50 p-3 rounded">
                <AlertTriangle className="h-4 w-4 mt-0.5 shrink-0" />
                <span>{restoreError}</span>
              </div>
            )}

            {restorePreview ? (
              <div className="space-y-3 bg-muted/30 p-4 rounded">
                <p className="text-sm font-medium">Backup contents</p>
                <div className="text-sm space-y-1 text-muted-foreground">
                  <p>Created: {new Date(restorePreview.createdAt).toLocaleString()}</p>
                  <p>Storages: {restorePreview.storageCount}</p>
                  {restorePreview.hasMcpSettings && <p>Includes MCP settings</p>}
                  {restorePreview.hasAppSettings && <p>Includes app settings</p>}
                </div>
                <div className="flex items-center gap-4 pt-1">
                  <label className="flex items-center gap-2 text-sm">
                    <Switch checked={restoreMcp} onCheckedChange={setRestoreMcp} />
                    Restore MCP settings
                  </label>
                  <label className="flex items-center gap-2 text-sm">
                    <Switch checked={restoreApp} onCheckedChange={setRestoreApp} />
                    Restore app settings
                  </label>
                </div>
                <div className="flex justify-end gap-3 pt-2">
                  <Button variant="outline" onClick={() => setRestorePreview(null)}>Back</Button>
                  <Button onClick={handleApplyRestore} disabled={restoring}>
                    {restoring ? "Restoring..." : "Apply Restore"}
                  </Button>
                </div>
              </div>
            ) : (
              <div className="flex justify-end gap-3 pt-2">
                <Button variant="outline" onClick={handleClose}>Cancel</Button>
                <Button
                  onClick={handlePreviewRestore}
                  disabled={!restoreArmored.trim() || !restorePassphrase}
                >
                  <Shield className="h-4 w-4 mr-1" /> Preview Restore
                </Button>
              </div>
            )}
          </TabsContent>
        </Tabs>
      </DialogContent>
    </Dialog>
  );
}
