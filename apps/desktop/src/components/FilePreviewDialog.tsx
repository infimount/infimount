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

  const [prevFileId, setPrevFileId] = useState<string | null>(null);

  if (file?.id !== prevFileId) {
    setPrevFileId(file?.id ?? null);
    setContent("");
    setError(null);
    setMode(null);
    setPreviewUrl(null);
    setLoading(true);

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
    if (!open || !file || file.type !== "file") return;
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
      setLoading(false);
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
          {!loading && !error && mode === "text" && (
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

const formatFileSize = (bytes: number) => {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  if (bytes < 1024 * 1024 * 1024) {
    return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
  }
  return `${(bytes / (1024 * 1024 * 1024)).toFixed(1)} GB`;
};
