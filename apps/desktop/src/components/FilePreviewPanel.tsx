import { useEffect, useState } from "react";
import { FileItem } from "@/types/storage";
import { X, Edit, Download } from "lucide-react";
import { Button } from "@/components/ui/button";
import { ScrollArea } from "@/components/ui/scroll-area";
import { getFileIcon, getFileColor } from "./FileIcon";
import { readFile } from "@/lib/api";

interface FilePreviewPanelProps {
  file: FileItem | null;
  onClose: () => void;
  onEdit?: () => void;
  onDownload?: () => void;
  sourceId: string;
}

export function FilePreviewPanel({
  file,
  onClose,
  onEdit,
  onDownload,
  sourceId,
}: FilePreviewPanelProps) {
  if (!file) return null;

  const Icon = getFileIcon(file);
  const color = getFileColor(file);
  const extension = file.extension?.toLowerCase();
  const isImage = extension
    ? ["jpg", "jpeg", "png", "gif", "webp", "svg"].includes(extension)
    : false;
  const isText = extension
    ? ["txt", "md", "json", "xml", "html", "css", "js", "ts"].includes(extension)
    : false;

  const [content, setContent] = useState<string>("");
  const [previewUrl, setPreviewUrl] = useState<string | null>(null);
  const [mode, setMode] = useState<"text" | "image" | "pdf" | "unsupported" | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);

  useEffect(() => {
    setContent("");
    setError(null);
    setMode(null);

    if (!file || file.type !== "file") return;

    const ext = (file.extension || file.name.split(".").pop() || "").toLowerCase();
    const isImageExt = ["jpg", "jpeg", "png", "gif", "webp", "svg"].includes(ext);
    const isPdfExt = ext === "pdf";

    let cancelled = false;

    setLoading(true);
    readFile(sourceId, file.id)
      .then((data) => {
        if (cancelled) return;

        if (isImageExt || isPdfExt) {
          const buffer = data.buffer.slice(
            data.byteOffset,
            data.byteOffset + data.byteLength,
          ) as ArrayBuffer;
          const mime =
            isImageExt && ext !== "svg"
              ? `image/${ext === "jpg" ? "jpeg" : ext}`
              : isImageExt
              ? "image/svg+xml"
              : "application/pdf";
          const blob = new Blob([buffer], { type: mime });
          const url = URL.createObjectURL(blob);
          setPreviewUrl(url);
          setMode(isImageExt ? "image" : "pdf");
          return;
        }

        // Fallback: try to treat as text
        try {
          const text = new TextDecoder().decode(data);
          setContent(text);
          setMode("text");
        } catch {
          setMode("unsupported");
          setError("Preview not available for this file type.");
        }
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
    <div className="flex h-full flex-col border-l border-border bg-card shadow-md">
      {/* Header */}
      <div className="flex items-center justify-between border-b p-4">
        <h3 className="flex-1 truncate font-semibold">{file.name}</h3>
        <Button size="icon" variant="ghost" onClick={onClose}>
          <X className="h-4 w-4" />
        </Button>
      </div>

      {/* Preview Area */}
      <ScrollArea className="flex-1 p-4">
        <div className="flex flex-col items-center gap-4">
          {loading && (
            <div className="flex min-h-[160px] items-center justify-center">
              <div className="flex flex-col items-center gap-2">
                <div className="h-6 w-6 animate-spin rounded-full border-2 border-primary border-t-transparent" />
                <span className="text-xs text-muted-foreground">
                  Loading preview…
                </span>
              </div>
            </div>
          )}
          {!loading && error && (
            <div className="flex min-h-[160px] items-center justify-center px-4 text-xs text-destructive">
              {error}
            </div>
          )}
          {!loading && !error && mode === "image" && previewUrl && (
            <div className="flex w-full items-center justify-center rounded-lg border bg-muted/20 p-4">
              <img
                src={previewUrl}
                alt={file.name}
                className="max-h-[60vh] max-w-full object-contain"
              />
            </div>
          )}
          {!loading && !error && mode === "pdf" && previewUrl && (
            <div className="w-full overflow-hidden rounded-lg border bg-muted/20">
              <iframe
                src={previewUrl}
                title={file.name}
                className="h-[60vh] w-full border-0"
              />
            </div>
          )}
          {!loading && !error && mode === "text" && content && (
            <div className="w-full rounded-lg border bg-muted/20 p-3 font-mono text-xs">
              <ScrollArea className="max-h-[60vh]">
                <pre className="whitespace-pre-wrap break-words">{content}</pre>
              </ScrollArea>
            </div>
          )}
          {!loading && !error && !mode && (
            <div className="flex flex-col items-center gap-4 py-8">
              <Icon className={`h-24 w-24 ${color}`} />
              <p className="text-sm text-muted-foreground">
                Preview not available
              </p>
            </div>
          )}
        </div>
      </ScrollArea>

      {/* File Info */}
      <div className="space-y-3 border-t p-4">
        <div className="text-sm">
          <div className="mb-2 font-medium">File Information</div>
          <div className="space-y-1 text-muted-foreground">
            <div className="flex justify-between">
              <span>Type:</span>
              <span>{file.extension?.toUpperCase() || "Unknown"}</span>
            </div>
            {typeof file.size === "number" && (
              <div className="flex justify-between">
                <span>Size:</span>
                <span>{formatFileSize(file.size)}</span>
              </div>
            )}
            <div className="flex justify-between">
              <span>Modified:</span>
              <span>
                {file.modified
                  ? file.modified.toLocaleDateString()
                  : "Unknown"}
              </span>
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
          {onEdit && (
            <Button
              size="sm"
              variant="outline"
              onClick={onEdit}
              className="flex-1"
            >
              <Edit className="mr-2 h-4 w-4" />
              Edit
            </Button>
          )}
          {onDownload && (
            <Button
              size="sm"
              variant="outline"
              onClick={onDownload}
              className="flex-1"
            >
              <Download className="mr-2 h-4 w-4" />
              Download
            </Button>
          )}
        </div>
      </div>
    </div>
  );
}

const formatFileSize = (bytes: number) => {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  if (bytes < 1024 * 1024 * 1024) return `${(bytes / (1024 * 1024)).toFixed(
    1,
  )} MB`;
  return `${(bytes / (1024 * 1024 * 1024)).toFixed(1)} GB`;
};
