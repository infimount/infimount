import { FileItem } from "@/types/storage";
import { X, Edit, Download } from "lucide-react";
import { Button } from "@/components/ui/button";
import { ScrollArea } from "@/components/ui/scroll-area";
import { getFileIcon, getFileColor } from "./FileIcon";

interface FilePreviewPanelProps {
  file: FileItem | null;
  onClose: () => void;
  onEdit: () => void;
  onDownload: () => void;
}

export function FilePreviewPanel({
  file,
  onClose,
  onEdit,
  onDownload,
}: FilePreviewPanelProps) {
  if (!file) return null;

  const Icon = getFileIcon(file);
  const color = getFileColor(file);
  const isImage =
    file.extension &&
    ["jpg", "jpeg", "png", "gif", "webp", "svg"].includes(
      file.extension.toLowerCase(),
    );
  const isText =
    file.extension &&
    ["txt", "md", "json", "xml", "html", "css", "js", "ts"].includes(
      file.extension.toLowerCase(),
    );

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
          {isImage ? (
            <div className="w-full rounded-lg border bg-muted/20 p-4">
              <img
                src={`https://via.placeholder.com/400x300?text=${file.name}`}
                alt={file.name}
                className="h-auto w-full rounded"
              />
              <div className="mt-4 space-y-1 rounded bg-background/50 p-3 text-xs">
                <p>
                  This is a sample image preview. In production, the actual image content
                  would be displayed here with full resolution and zoom capabilities.
                </p>
                <p className="text-muted-foreground">Image dimensions: 400x300px</p>
              </div>
            </div>
          ) : isText ? (
            <div className="w-full rounded-lg border bg-muted/20 p-4 font-mono text-sm">
              <pre className="whitespace-pre-wrap">
{`// Preview of ${file.name}
// This is a mock preview demonstrating text file display

function exampleCode() {
  console.log("Hello, World!");
  return {
    status: "success",
    message: "This is sample content"
  };
}

// In production, actual file content would be loaded here
// with syntax highlighting for code files and proper
// formatting for markdown, JSON, and other text formats.

const data = {
  features: ["Syntax highlighting", "Line numbers", "Code folding"],
  supported: ["JavaScript", "TypeScript", "Python", "JSON", "Markdown"]
};`}
              </pre>
            </div>
          ) : (
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
