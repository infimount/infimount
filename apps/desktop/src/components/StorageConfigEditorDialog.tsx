import { useEffect, useState } from "react";
import { Braces, RefreshCw } from "lucide-react";

import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Button } from "@/components/ui/button";
import { JsonCodeEditor } from "@/components/JsonCodeEditor";

interface StorageConfigEditorDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onLoad: () => Promise<string>;
  onSave: (json: string) => Promise<void>;
}

export function StorageConfigEditorDialog({
  open,
  onOpenChange,
  onLoad,
  onSave,
}: StorageConfigEditorDialogProps) {
  const [jsonText, setJsonText] = useState("[]");
  const [error, setError] = useState<string | null>(null);
  const [isLoading, setIsLoading] = useState(false);
  const [isSaving, setIsSaving] = useState(false);

  const loadJson = async () => {
    setIsLoading(true);
    setError(null);
    try {
      const json = await onLoad();
      setJsonText(json);
    } catch (loadError) {
      setError(loadError instanceof Error ? loadError.message : String(loadError));
    } finally {
      setIsLoading(false);
    }
  };

  useEffect(() => {
    if (!open) return;
    void loadJson();
  }, [open]);

  const handleFormat = () => {
    try {
      const parsed = JSON.parse(jsonText);
      setJsonText(JSON.stringify(parsed, null, 2));
      setError(null);
    } catch (formatError) {
      setError(formatError instanceof Error ? formatError.message : String(formatError));
    }
  };

  const handleSave = async () => {
    setIsSaving(true);
    setError(null);
    try {
      await onSave(jsonText);
      onOpenChange(false);
    } catch (saveError) {
      setError(saveError instanceof Error ? saveError.message : String(saveError));
    } finally {
      setIsSaving(false);
    }
  };

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-[840px] max-h-[88vh] overflow-y-auto rounded-2xl border border-border bg-background text-foreground shadow-2xl">
        <DialogHeader>
          <DialogTitle className="text-left text-base font-normal text-[hsl(var(--card-foreground))]">
            Edit Storage Config JSON
          </DialogTitle>
          <DialogDescription className="text-left text-xs text-muted-foreground">
            This is the full combined storage registry. Saving replaces the current registry using the edited JSON.
          </DialogDescription>
        </DialogHeader>

        <div className="space-y-4">
          <div className="flex items-center justify-between gap-3">
            <p className="text-[11px] text-muted-foreground">
              Secret values are visible here because this editor works on the complete exported registry.
            </p>
            <div className="flex items-center gap-2">
              <Button
                type="button"
                variant="outline"
                className="border border-border hover:bg-sidebar-accent/30 hover:text-foreground"
                onClick={() => void loadJson()}
                disabled={isLoading}
              >
                <RefreshCw className="mr-2 h-4 w-4" />
                Reload
              </Button>
              <Button
                type="button"
                variant="outline"
                className="border border-border hover:bg-sidebar-accent/30 hover:text-foreground"
                onClick={handleFormat}
              >
                <Braces className="mr-2 h-4 w-4" />
                Format JSON
              </Button>
            </div>
          </div>

          <JsonCodeEditor
            value={jsonText}
            minHeight="420px"
            onChange={(value) => {
              setJsonText(value);
              setError(null);
            }}
          />

          {error ? (
            <div className="rounded-md border border-rose-300/80 bg-rose-50 px-3 py-2 text-xs text-rose-700 dark:border-rose-700/60 dark:bg-rose-950/40 dark:text-rose-300">
              {error}
            </div>
          ) : null}

          <div className="flex justify-end gap-3 pt-2">
            <Button
              type="button"
              variant="outline"
              className="border border-border hover:bg-sidebar-accent/30 hover:text-foreground"
              onClick={() => onOpenChange(false)}
            >
              Close
            </Button>
            <Button
              type="button"
              className="bg-primary text-primary-foreground hover:bg-primary/90"
              onClick={handleSave}
              disabled={isLoading || isSaving}
            >
              {isSaving ? "Saving..." : "Apply JSON"}
            </Button>
          </div>
        </div>
      </DialogContent>
    </Dialog>
  );
}
