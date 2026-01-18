import { useEffect, useState } from "react";
import { FileItem } from "@/types/storage";
import { X, Edit, Download } from "lucide-react";
import { Button } from "@/components/ui/button";
import { ScrollArea } from "@/components/ui/scroll-area";
import { getFileIcon, getFileColor } from "./FileIcon";
import { readFile } from "@/lib/api";

interface FilePreviewPanelProps {
  file: FileItem | null;
  sourceId: string;
  onClose: () => void;
  onEdit: () => void;
  onDownload: () => void;
}

export function FilePreviewPanel({
  file,
  sourceId,
  onClose,
  onEdit,
  onDownload,
}: FilePreviewPanelProps) {
  if (!file) return null;

  const Icon = getFileIcon(file);
  const color = getFileColor(file);

  const [content, setContent] = useState<string>("");
  const [previewUrl, setPreviewUrl] = useState<string | null>(null);
  const [mode, setMode] = useState<"text" | "image" | "pdf" | "unsupported" | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);

  useEffect(() => {
    setContent("");
    setError(null);
    setMode(null);

    if (!file || file.type !== "file") {
      // Directories or missing file: no preview, just show icon/info block.
      return;
    }

    const ext =
      (file.extension || file.name.split(".").pop() || "").toLowerCase();
    const isImage = ["jpg", "jpeg", "png", "gif", "webp", "svg"].includes(ext);
    const isPdf = ext === "pdf";
    const isTextExt = [
      "txt",
      "md",
      "json",
      "xml",
      "html",
      "css",
      "js",
      "ts",
      "tsx",
      "jsx",
      "log",
      "csv",
    ].includes(ext);
    const isKnownBinary = [
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
    ].includes(ext);

    // Short-circuit for binary/unsupported types: don't even read the file.
    if (isKnownBinary || (!isImage && !isPdf && !isTextExt)) {
      setMode("unsupported");
      setError("Preview not available for this file type.");
      return;
    }

    let cancelled = false;

    setLoading(true);
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

        // Text preview
        const text = new TextDecoder().decode(data);
        setContent(text);
        setMode("text");
      })
      .catch((e: any) => {
        if (cancelled) return;
        setMode("unsupported");
        setError(e?.message || String(e));
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });

    return () => {
      cancelled = true;
    };
  }, [file, sourceId]);

  useEffect(() => {
    return () => {
      if (previewUrl) {
        URL.revokeObjectURL(previewUrl);
      }
    };
  }, [previewUrl]);

  return (
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
        <div className="p-4">
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
          {!loading && !error && mode === "text" && content && (
            <div className="w-full rounded-lg border bg-muted/20 p-3 font-mono text-xs">
              <pre className="whitespace-pre-wrap break-words">{content}</pre>
            </div>
          )}
          {!loading && !error && (!mode || mode === "unsupported") && (
            <div className="flex flex-col items-center gap-4 py-8">
              <Icon className={`h-24 w-24 ${color}`} />
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
      <div className="space-y-3 bg-muted/30 p-4">
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
          <Button size="sm" variant="outline" onClick={onEdit} className="flex-1">
            <Edit className="mr-2 h-4 w-4" />
            Edit
          </Button>
          <Button size="sm" variant="outline" onClick={onDownload} className="flex-1">
            <Download className="mr-2 h-4 w-4" />
            Download
          </Button>
        </div>
      </div>
    </div>
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
