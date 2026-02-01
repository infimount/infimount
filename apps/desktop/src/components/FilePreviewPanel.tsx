import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { FileItem } from "@/types/storage";
import { X, Download, Edit3, Save, Undo2 } from "lucide-react";
import { Button } from "@/components/ui/button";
import { ScrollArea } from "@/components/ui/scroll-area";
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/components/ui/alert-dialog";
import { FileTypeIcon } from "./FileIcon";
import { readFile, statEntry, writeFile } from "@/lib/api";
import { toast } from "@/hooks/use-toast";

const MAX_PREVIEW_BYTES = 20 * 1024 * 1024;

const TEXT_EXTENSIONS = new Set([
  "txt",
  "md",
  "mdx",
  "rst",
  "rtf",
  "json",
  "jsonl",
  "ndjson",
  "xml",
  "html",
  "css",
  "js",
  "ts",
  "tsx",
  "jsx",
  "mjs",
  "cjs",
  "vue",
  "svelte",
  "log",
  "csv",
  "tsv",
  "toml",
  "yaml",
  "yml",
  "env",
  "ini",
  "conf",
  "cfg",
  "properties",
  "gradle",
  "groovy",
  "rs",
  "go",
  "py",
  "rb",
  "pl",
  "pm",
  "lua",
  "php",
  "java",
  "kt",
  "kts",
  "scala",
  "cs",
  "fs",
  "fsx",
  "swift",
  "c",
  "h",
  "cpp",
  "hpp",
  "sql",
  "proto",
  "graphql",
  "gql",
  "eml",
  "ics",
  "vcf",
  "srt",
  "vtt",
  "ass",
  "sh",
  "bash",
  "zsh",
  "ps1",
  "bat",
  "cmd",
  "diff",
  "patch",
]);

const TEXT_FILENAMES = new Set([
  "dockerfile",
  "makefile",
  "cmakelists.txt",
  "readme",
  "readme.md",
  "readme.txt",
  "license",
  "license.txt",
  "license.md",
  ".gitignore",
  ".gitattributes",
  ".gitmodules",
  ".editorconfig",
]);

const BINARY_EXTENSIONS = new Set([
  "zip",
  "rar",
  "7z",
  "tar",
  "gz",
  "tgz",
  "bz2",
  "xz",
  "exe",
  "dll",
  "bin",
  "iso",
  "dmg",
  "pkg",
  "deb",
  "rpm",
  "msi",
  "msg",
  "safetensors",
  "pt",
  "pth",
  "ckpt",
  "onnx",
  "npy",
  "npz",
  "pkl",
  "pickle",
]);

const isLikelyText = (data: Uint8Array): boolean => {
  const sample = data.subarray(0, 4096);
  if (sample.length === 0) return true;
  let nonPrintable = 0;
  for (const byte of sample) {
    if (byte === 0) return false;
    if (byte < 9 || (byte > 13 && byte < 32)) {
      nonPrintable += 1;
    }
  }
  return nonPrintable / sample.length < 0.08;
};

interface FilePreviewPanelProps {
  file: FileItem | null;
  sourceId: string;
  onClose: () => void;
  startInEditMode?: boolean;
  onEditModeChange?: (editing: boolean) => void;
  onDownload: () => void;
}

export function FilePreviewPanel({
  file,
  sourceId,
  onClose,
  startInEditMode,
  onEditModeChange,
  onDownload,
}: FilePreviewPanelProps) {
  const [content, setContent] = useState<string>("");
  const [previewUrl, setPreviewUrl] = useState<string | null>(null);
  const [mode, setMode] = useState<"text" | "image" | "pdf" | "unsupported" | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const [isEditing, setIsEditing] = useState(false);
  const [draftContent, setDraftContent] = useState("");
  const [originalContent, setOriginalContent] = useState("");
  const [isSaving, setIsSaving] = useState(false);
  const [showOverwriteConfirm, setShowOverwriteConfirm] = useState(false);
  const [editBaselineMs, setEditBaselineMs] = useState<number | null>(null);
  const [editBaselineRaw, setEditBaselineRaw] = useState<string | null>(null);
  const [remoteModifiedAtLabel, setRemoteModifiedAtLabel] = useState<string | null>(null);
  const editorRef = useRef<HTMLTextAreaElement | null>(null);

  const [prevFileId, setPrevFileId] = useState<string | null>(null);

  if (file?.id !== prevFileId) {
    setPrevFileId(file?.id ?? null);
    setContent("");
    setError(null);
    setMode(null);
    setPreviewUrl(null);
    setLoading(true);
    setIsEditing(false);
    setDraftContent("");
    setOriginalContent("");
    setEditBaselineMs(null);
    setEditBaselineRaw(null);
    setRemoteModifiedAtLabel(null);

    if (file && file.type === "file") {
      const ext = (file.extension || file.name.split(".").pop() || "").toLowerCase();
      const isKnownBinary = BINARY_EXTENSIONS.has(ext);

      if (file.size && file.size > MAX_PREVIEW_BYTES) {
        setMode("unsupported");
        setError(`File is too large to preview (${formatFileSize(file.size)}).`);
      } else if (isKnownBinary) {
        setMode("unsupported");
        setError("Preview not available for this file type.");
      }
    }
  }

  useEffect(() => {
    if (!file || file.type !== "file") {
      // Directories or missing file: no preview, just show icon/info block.
      return;
    }

    if (mode === "unsupported") return;

    const ext = (file.extension || file.name.split(".").pop() || "").toLowerCase();
    const isImage = ["jpg", "jpeg", "png", "gif", "webp", "svg"].includes(ext);
    const isPdf = ext === "pdf";
    const lowerName = file.name.toLowerCase();
    const isTextExt =
      TEXT_EXTENSIONS.has(ext) ||
      TEXT_FILENAMES.has(lowerName) ||
      lowerName.startsWith(".env") ||
      lowerName.startsWith("dockerfile") ||
      lowerName.startsWith("makefile");

    let cancelled = false;

    // setLoading(true); // Moved to render phase reset
    readFile(sourceId, file.id)
      .then((data) => {
        if (cancelled) return;

        if (isImage || isPdf) {
          const buffer = data.buffer.slice(
            data.byteOffset,
            data.byteOffset + data.byteLength,
          ) as ArrayBuffer;
          const mime =
            isImage && ext !== "svg"
              ? `image/${ext === "jpg" ? "jpeg" : ext}`
              : isImage
                ? "image/svg+xml"
                : "application/pdf";
          const blob = new Blob([buffer], { type: mime });
          const url = URL.createObjectURL(blob);
          setPreviewUrl(url);
          setMode(isImage ? "image" : "pdf");
          return;
        }

        const dataIsText = isTextExt || isLikelyText(data);
        if (!dataIsText) {
          setMode("unsupported");
          setError("Preview not available for this file type.");
          return;
        }

        const text = new TextDecoder().decode(data);
        setContent(text);
        setDraftContent(text);
        setOriginalContent(text);
        setMode("text");
      })
      .catch((e: unknown) => {
        if (cancelled) return;
        setMode("unsupported");
        setError(e instanceof Error ? e.message : String(e));
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });

    return () => {
      cancelled = true;
      setLoading(false);
    };
  }, [file, sourceId]);

  const isDirty = isEditing && draftContent !== originalContent;
  const canEdit = mode === "text" && !loading && !error;

  useEffect(() => {
    if (startInEditMode && !loading && !canEdit) {
      onEditModeChange?.(false);
    }
  }, [startInEditMode, loading, canEdit, onEditModeChange]);

  const beginEdit = useCallback(async () => {
    if (isEditing || loading || error || mode !== "text" || !file || file.type !== "file") {
      return;
    }
    setIsEditing(true);
    onEditModeChange?.(true);

    let baselineMs: number | null = null;
    let baselineRaw: string | null = null;
    try {
      const latest = await statEntry(sourceId, file.id);
      baselineRaw = latest.modified_at ?? null;
      if (latest.modified_at) {
        const parsed = Date.parse(latest.modified_at);
        baselineMs = Number.isNaN(parsed) ? null : parsed;
      }
    } catch {
      baselineMs = file.modified?.getTime() ?? null;
      baselineRaw = file.modified ? file.modified.toISOString() : null;
    }

    setEditBaselineMs(baselineMs);
    setEditBaselineRaw(baselineRaw);
  }, [isEditing, loading, error, mode, file, sourceId, onEditModeChange]);

  const attemptSave = useCallback(
    async (force: boolean) => {
      if (!file || file.type !== "file") return;
      if (draftContent === originalContent || isSaving) return;
      setIsSaving(true);

      let latestModifiedMs: number | null = null;
      let latestModifiedLabel: string | null = null;
      let latestModifiedRaw: string | null = null;
      try {
        const latest = await statEntry(sourceId, file.id);
        if (latest.modified_at) {
          latestModifiedRaw = latest.modified_at;
          const parsed = Date.parse(latest.modified_at);
          if (!Number.isNaN(parsed)) {
            latestModifiedMs = parsed;
            latestModifiedLabel = new Date(parsed).toLocaleString();
          } else {
            latestModifiedLabel = latest.modified_at;
          }
        }
      } catch {
        latestModifiedMs = null;
      }

      const hasRawConflict =
        editBaselineRaw && latestModifiedRaw && latestModifiedRaw !== editBaselineRaw;
      const hasMsConflict =
        !hasRawConflict &&
        editBaselineMs !== null &&
        latestModifiedMs !== null &&
        latestModifiedMs !== editBaselineMs;

      if (!force && (hasRawConflict || hasMsConflict)) {
        setRemoteModifiedAtLabel(latestModifiedLabel ?? latestModifiedRaw);
        setShowOverwriteConfirm(true);
        setIsSaving(false);
        return;
      }

      try {
        const data = new TextEncoder().encode(draftContent);
        await writeFile(sourceId, file.id, data);
        setContent(draftContent);
        setOriginalContent(draftContent);
        setIsEditing(false);
        setEditBaselineMs(null);
        setEditBaselineRaw(null);
        onEditModeChange?.(false);

        try {
          const updated = await statEntry(sourceId, file.id);
          if (updated.modified_at) {
            const parsed = Date.parse(updated.modified_at);
            setEditBaselineMs(Number.isNaN(parsed) ? null : parsed);
            setEditBaselineRaw(updated.modified_at);
          }
        } catch {
          setEditBaselineMs(null);
          setEditBaselineRaw(null);
        }

        toast({
          title: "Saved",
          description: `${file.name} updated successfully.`,
          variant: "success",
          duration: 2000,
        });
      } catch (err: unknown) {
        toast({
          title: "Save failed",
          description: err instanceof Error ? err.message : String(err),
          variant: "destructive",
        });
      } finally {
        setIsSaving(false);
      }
    },
    [
      file,
      draftContent,
      originalContent,
      isSaving,
      sourceId,
      editBaselineRaw,
      editBaselineMs,
      onEditModeChange,
    ],
  );

  const lineNumbers = useMemo(() => {
    if (!isEditing) return "";
    let count = 1;
    for (let i = 0; i < draftContent.length; i += 1) {
      if (draftContent[i] === "\n") count += 1;
    }
    return Array.from({ length: count }, (_, i) => i + 1).join("\n");
  }, [draftContent, isEditing]);

  useEffect(() => {
    if (!startInEditMode || isEditing || loading || error || mode !== "text") {
      return;
    }
    void beginEdit();
  }, [startInEditMode, isEditing, loading, error, mode, beginEdit]);

  useEffect(() => {
    if (isEditing) {
      editorRef.current?.focus();
    }
  }, [isEditing]);

  useEffect(() => {
    return () => {
      if (previewUrl) {
        URL.revokeObjectURL(previewUrl);
      }
    };
  }, [previewUrl]);

  if (!file) return null;

  return (
    <>
      <div className="flex h-full flex-col bg-card">
        {/* Header */}
        <div className="flex items-center justify-between bg-muted/30 p-3">
          <h3 className="flex-1 truncate text-sm font-semibold">{file.name}</h3>
          <Button size="icon" variant="ghost" onClick={onClose} className="h-8 w-8">
            <X className="h-4 w-4" />
          </Button>
        </div>

        {/* Preview Area */}
        <ScrollArea className="flex-1">
          <div className="min-h-full">
            {loading && (
              <div className="flex h-full flex-col items-center justify-center gap-2 text-xs text-muted-foreground">
                <div className="h-5 w-5 animate-spin rounded-full border-2 border-primary border-t-transparent" />
                <span>Loading preview…</span>
              </div>
            )}
            {!loading && error && (
              <div className="flex h-full items-center justify-center px-4 text-xs text-destructive">
                {error}
              </div>
            )}
            {!loading && !error && mode === "image" && previewUrl && (
              <div className="flex w-full items-center justify-center rounded-lg border bg-muted/20 p-2">
                <img
                  src={previewUrl}
                  alt={file.name}
                  className="max-h-[360px] w-auto max-w-full rounded"
                />
              </div>
            )}
            {!loading && !error && mode === "pdf" && previewUrl && (
              <div className="w-full overflow-hidden rounded-lg border bg-muted/20">
                <iframe
                  src={previewUrl}
                  title={file.name}
                  className="h-[360px] w-full border-0"
                />
              </div>
            )}
            {!loading && !error && mode === "text" && !isEditing && (
              <div className="w-full h-full p-4 font-mono text-xs overflow-x-hidden">
                <pre className="whitespace-pre-wrap break-words">{content}</pre>
              </div>
            )}
            {!loading && !error && mode === "text" && isEditing && (
              <div className="w-full h-full">
                <div className="flex h-[360px] w-full overflow-hidden text-xs font-mono">
                  <pre className="select-none border-r bg-muted/30 px-2 py-3 text-muted-foreground">
                    {lineNumbers}
                  </pre>
                  <textarea
                    ref={editorRef}
                    value={draftContent}
                    onChange={(event) => setDraftContent(event.target.value)}
                    onKeyDown={(event) => {
                      if ((event.metaKey || event.ctrlKey) && event.key === "s") {
                        event.preventDefault();
                        void attemptSave(false);
                      }
                    }}
                    className="h-full w-full resize-none bg-transparent px-3 py-3 outline-none whitespace-pre overflow-x-auto"
                    spellCheck={false}
                  />
                </div>
              </div>
            )}
            {!loading && !error && (!mode || mode === "unsupported") && (
              <div className="flex flex-col items-center gap-4 py-8">
                <FileTypeIcon item={file} className="h-24 w-24" />
                <p className="text-sm text-muted-foreground">Preview not available</p>
                <p className="max-w-xs text-center text-xs text-muted-foreground">
                  This file type cannot be previewed in the browser. Use the download
                  button below to save it to your device.
                </p>
              </div>
            )}
          </div>
        </ScrollArea>

        {/* File Info */}
        <div className="space-y-3 border-t border-border/60 bg-background/50 backdrop-blur supports-[backdrop-filter]:bg-background/20 p-4">
          <div className="text-sm">
            <div className="mb-2 font-medium">File Information</div>
            <div className="space-y-1 text-muted-foreground">
              <div className="flex justify-between">
                <span>Type:</span>
                <span>{file.extension?.toUpperCase() || "Unknown"}</span>
              </div>
              {file.size && (
                <div className="flex justify-between">
                  <span>Size:</span>
                  <span>{formatFileSize(file.size)}</span>
                </div>
              )}
              <div className="flex justify-between">
                <span>Modified:</span>
                <span>{file.modified?.toLocaleDateString() ?? "Unknown"}</span>
              </div>
              {file.owner && (
                <div className="flex justify-between">
                  <span>Owner:</span>
                  <span>{file.owner}</span>
                </div>
              )}
            </div>
          </div>

          {/* Action Buttons */}
          <div className="flex gap-2">
            {canEdit && !isEditing && (
              <Button
                size="sm"
                variant="outline"
                onClick={() => void beginEdit()}
                className="flex-1"
              >
                <Edit3 className="mr-2 h-4 w-4" />
                Edit
              </Button>
            )}
            {canEdit && isEditing && (
              <>
                <Button
                  size="sm"
                  variant="outline"
                  className="flex-1"
                  onClick={() => {
                    setDraftContent(originalContent);
                    setIsEditing(false);
                    setEditBaselineMs(null);
                    setEditBaselineRaw(null);
                    onEditModeChange?.(false);
                  }}
                  disabled={isSaving}
                >
                  <Undo2 className="mr-2 h-4 w-4" />
                  Discard
                </Button>
                <Button
                  size="sm"
                  className="flex-1"
                  onClick={() => void attemptSave(false)}
                  disabled={!isDirty || isSaving}
                >
                  <Save className="mr-2 h-4 w-4" />
                  {isSaving ? "Saving..." : "Save"}
                </Button>
              </>
            )}
            {!isEditing && (
              <Button size="sm" variant="outline" onClick={onDownload} className="flex-1">
                <Download className="mr-2 h-4 w-4" />
                Download
              </Button>
            )}
          </div>
        </div>
      </div>

      <AlertDialog open={showOverwriteConfirm} onOpenChange={setShowOverwriteConfirm}>
        <AlertDialogContent className="max-w-md rounded-2xl border border-border bg-[hsl(var(--card))] text-[hsl(var(--card-foreground))] shadow-2xl">
          <AlertDialogHeader>
            <AlertDialogTitle>File changed on disk</AlertDialogTitle>
            <AlertDialogDescription>
              This file was modified after you opened it.
              {remoteModifiedAtLabel && (
                <>
                  {" "}
                  Latest change: {remoteModifiedAtLabel}.
                </>
              )}{" "}
              Do you want to overwrite it with your changes?
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel
              onClick={() => setShowOverwriteConfirm(false)}
              disabled={isSaving}
            >
              Cancel
            </AlertDialogCancel>
            <AlertDialogAction
              className="bg-destructive text-destructive-foreground hover:bg-destructive/90"
              onClick={() => {
                setShowOverwriteConfirm(false);
                void attemptSave(true);
              }}
            >
              Overwrite
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </>
  );
}

const formatFileSize = (bytes: number) => {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  if (bytes < 1024 * 1024 * 1024) {
    return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
  }
  return `${(bytes / (1024 * 1024 * 1024)).toFixed(1)} GB`;
};
