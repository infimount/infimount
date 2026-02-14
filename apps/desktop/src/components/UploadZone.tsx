import { useCallback, useState, forwardRef, useImperativeHandle } from "react";
import type React from "react";
import { Upload, X } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Progress } from "@/components/ui/progress";

// Minimal interface we need for uploads.
export interface UploadFileLike {
  name: string;
  arrayBuffer: () => Promise<ArrayBuffer>;
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
    const [uploadProgress, setUploadProgress] = useState<
      { name: string; progress: number }[]
    >([]);

    const handleFiles = useCallback((files: UploadFileLike[]) => {
      const progressData = files.map((file) => ({ name: file.name, progress: 0 }));
      setUploadProgress(progressData);

      files.forEach((_, index) => {
        const interval = setInterval(() => {
          setUploadProgress((previous) => {
            const updated = [...previous];
            if (!updated[index]) {
              clearInterval(interval);
              return previous;
            }
            if (updated[index].progress < 100) {
              updated[index].progress += 10;
            } else {
              clearInterval(interval);
            }
            return updated;
          });
        }, 200);
      });

      setTimeout(() => {
        onUpload(files);
        setUploadProgress([]);
      }, 2500);
    }, [onUpload]);

    useImperativeHandle(ref, () => ({
      handleFiles,
    }));

    const handleFileSelect = useCallback(
      (event: React.ChangeEvent<HTMLInputElement>) => {
        const rawFiles = event.target.files ? Array.from(event.target.files) : [];
        if (!rawFiles.length) return;

        const fileLikes: UploadFileLike[] = rawFiles.map((file) => {
          const anyFile = file as any;
          const relPath: string =
            typeof anyFile.webkitRelativePath === "string" && anyFile.webkitRelativePath.length > 0
              ? anyFile.webkitRelativePath
              : file.name;

          return {
            name: relPath,
            arrayBuffer: () => file.arrayBuffer(),
          };
        });

        handleFiles(fileLikes);
        // Allow selecting the same file again in subsequent picks.
        event.target.value = "";
      },
      [handleFiles],
    );

    const cancelUpload = (index: number) => {
      setUploadProgress((previous) => previous.filter((_, i) => i !== index));
    };

    if (uploadProgress.length > 0) {
      return (
        <div className="absolute inset-0 z-40 flex items-center justify-center bg-background/95 p-8 backdrop-blur-sm">
          <div className="w-full max-w-md space-y-4">
            <h3 className="text-lg font-semibold">Uploading Files</h3>
            {uploadProgress.map((item, index) => (
              <div key={item.name + index} className="space-y-2">
                <div className="flex items-center justify-between text-sm">
                  <span className="flex-1 truncate">{item.name}</span>
                  <Button
                    size="icon"
                    variant="ghost"
                    className="h-6 w-6"
                    onClick={() => cancelUpload(index)}
                  >
                    <X className="h-4 w-4" />
                  </Button>
                </div>
                <Progress value={item.progress} />
              </div>
            ))}
          </div>
        </div>
      );
    }

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
