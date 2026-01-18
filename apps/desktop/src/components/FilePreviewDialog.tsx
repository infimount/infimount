import { useEffect, useState } from "react";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogDescription,
} from "@/components/ui/dialog";
import type { FileItem } from "@/types/storage";
import { readFile } from "@/lib/api";

interface FilePreviewDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  file: FileItem | null;
  sourceId: string;
}

export const FilePreviewDialog: React.FC<FilePreviewDialogProps> = ({
  open,
  onOpenChange,
  file,
  sourceId,
}) => {
  const [content, setContent] = useState<string>("");
  const [previewUrl, setPreviewUrl] = useState<string | null>(null);
  const [mode, setMode] = useState<"text" | "image" | "pdf" | "unsupported" | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);

  useEffect(() => {
    setContent("");
    setError(null);
    setMode(null);

    if (!open || !file || file.type !== "file") return;

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
  }, [open, file, sourceId]);

  useEffect(() => {
    // Cleanup object URL when preview changes or dialog unmounts
    return () => {
      if (previewUrl) {
        URL.revokeObjectURL(previewUrl);
      }
    };
  }, [previewUrl]);

  return (
    <Dialog
      open={open}
      onOpenChange={onOpenChange}
    >
      <DialogContent className="sm:max-w-2xl h-[70vh] flex flex-col rounded-2xl border border-border bg-[hsl(var(--card))] text-[hsl(var(--card-foreground))] shadow-2xl">
        <DialogHeader>
          <DialogTitle className="truncate">{file?.name ?? "Preview"}</DialogTitle>
          {file && (
            <DialogDescription className="truncate text-xs">
              {file.id}
            </DialogDescription>
          )}
        </DialogHeader>
        <div className="mt-3 flex-1 overflow-hidden rounded-md border border-border bg-[hsl(var(--background))]">
          {loading && (
            <div className="flex h-full flex-col items-center justify-center gap-3">
              <div className="h-6 w-6 animate-spin rounded-full border-2 border-primary border-t-transparent" />
              <span className="text-xs text-muted-foreground">
                Loading preview…
              </span>
            </div>
          )}
          {!loading && error && (
            <div className="flex h-full items-center justify-center px-4 text-xs text-destructive">
              {error}
            </div>
          )}
          {!loading && !error && mode === "image" && previewUrl && (
            <div className="flex h-full w-full items-center justify-center bg-[hsl(var(--background))]">
              <img
                src={previewUrl}
                alt={file?.name ?? "Image preview"}
                className="max-h-full max-w-full object-contain"
              />
            </div>
          )}
          {!loading && !error && mode === "pdf" && previewUrl && (
            <iframe
              src={previewUrl}
              title={file?.name ?? "PDF preview"}
              className="h-full w-full border-0"
            />
          )}
          {!loading && !error && mode === "text" && content && (
            <div className="h-full w-full overflow-auto p-3 text-xs font-mono whitespace-pre-wrap break-words">
              <pre>{content}</pre>
            </div>
          )}
          {!loading && !error && mode === "unsupported" && (
            <div className="flex h-full items-center justify-center px-4 text-xs text-muted-foreground">
              Preview not available for this file type.
            </div>
          )}
          {!loading && !error && !mode && (
            <div className="flex h-full items-center justify-center px-4 text-xs text-muted-foreground">
              Empty file or unsupported preview format.
            </div>
          )}
        </div>
      </DialogContent>
    </Dialog>
  );
};
