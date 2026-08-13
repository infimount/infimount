import { useCallback, forwardRef, useImperativeHandle } from "react";
import type React from "react";
import { Upload } from "lucide-react";

// Minimal interface we need for uploads.
export interface UploadFileLike {
  name: string;
  size?: number;
  arrayBuffer: () => Promise<ArrayBuffer>;
  slice?: (start?: number, end?: number) => Blob;
}

export interface UploadZoneRef {
  handleFiles: (files: UploadFileLike[]) => void;
}

interface UploadZoneProps {
  onUpload: (files: UploadFileLike[]) => void;
  isDragging?: boolean;
}

export const UploadZone = forwardRef<UploadZoneRef, UploadZoneProps>(
  ({ onUpload, isDragging }, ref) => {
    const handleFiles = useCallback((files: UploadFileLike[]) => {
      if (files.length === 0) return;
      onUpload(files);
    }, [onUpload]);

    useImperativeHandle(ref, () => ({
      handleFiles,
    }));

    const handleFileSelect = useCallback(
      (event: React.ChangeEvent<HTMLInputElement>) => {
        const rawFiles = event.target.files ? Array.from(event.target.files) : [];
        if (!rawFiles.length) return;

        const fileLikes: UploadFileLike[] = rawFiles.map((file) => {
          const relPath: string =
            typeof file.webkitRelativePath === "string" && file.webkitRelativePath.length > 0
              ? file.webkitRelativePath
              : file.name;

          return {
            name: relPath,
            size: file.size,
            arrayBuffer: () => file.arrayBuffer(),
            slice: (start, end) => file.slice(start, end),
          };
        });

        handleFiles(fileLikes);
        // Allow selecting the same file again in subsequent picks.
        event.target.value = "";
      },
      [handleFiles],
    );

    return (
      <div
        className={`pointer-events-none absolute inset-0 z-30 transition-all ${isDragging ? "bg-primary/10 backdrop-blur-sm" : ""
          }`}
      >
        {isDragging && (
          <div className="flex h-full items-center justify-center">
            <div className="rounded-lg border-2 border-dashed border-primary bg-card p-12">
              <div className="flex flex-col items-center gap-4">
                <Upload className="h-16 w-16 text-primary" />
                <p className="text-lg font-medium">Drop files here to upload</p>
              </div>
            </div>
          </div>
        )}
        <input
          id="file-upload"
          type="file"
          multiple
          onChange={handleFileSelect}
          className="hidden"
        />
      </div>
    );
  }
);
UploadZone.displayName = "UploadZone";
