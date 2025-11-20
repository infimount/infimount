import { FileItem } from "@/types/storage";
import { MoreVertical, Edit, Eye, Download, Trash2 } from "lucide-react";
import { getFileIcon, getFileColor } from "./FileIcon";
import { Card } from "@/components/ui/card";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";

const formatFileSize = (bytes?: number) => {
  if (!bytes) return "";
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  if (bytes < 1024 * 1024 * 1024) return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
  return `${(bytes / (1024 * 1024 * 1024)).toFixed(1)} GB`;
};

interface FileGridProps {
  files: FileItem[];
  selectedFiles: Set<string>;
  onSelectFile: (fileId: string) => void;
  onOpenFile?: (file: FileItem) => void;
  onDownloadFile?: (file: FileItem) => void;
  onDeleteFile?: (file: FileItem) => void;
}

export function FileGrid({
  files,
  selectedFiles,
  onSelectFile,
  onOpenFile,
  onDownloadFile,
  onDeleteFile,
}: FileGridProps) {
  return (
    <div className="grid grid-cols-3 gap-3 sm:grid-cols-4 sm:gap-4 lg:grid-cols-5 xl:grid-cols-6">
      {files.map((file) => {
        const Icon = getFileIcon(file);
        const color = getFileColor(file);
        const isSelected = selectedFiles.has(file.id);

        return (
          <Card
            key={file.id}
            className={`group relative cursor-pointer overflow-hidden transition-all hover:shadow-md ${
              isSelected ? "border-primary ring-2 ring-primary/20" : "hover:border-primary/50"
            }`}
            onDoubleClick={() => onOpenFile?.(file)}
          >
            <div className="flex flex-col items-center gap-3 p-3 sm:p-4">
              <div className="relative w-full">
                <Checkbox
                  checked={isSelected}
                  onCheckedChange={() => onSelectFile(file.id)}
                  className="absolute left-0 top-0 z-10 opacity-0 group-hover:opacity-100 data-[state=checked]:opacity-100"
                  onClick={(e) => e.stopPropagation()}
                />
                <div className="flex justify-center">
                  <Icon className={`h-8 w-8 sm:h-10 sm:w-10 lg:h-12 lg:w-12 ${color}`} />
                </div>
              </div>

              <div className="w-full text-center">
                <p className="mx-auto max-w-[14ch] truncate text-sm font-medium" title={file.name}>
                  {file.name}
                </p>
                {file.size && (
                  <p className="mt-1 text-xs text-muted-foreground">
                    {formatFileSize(file.size)}
                  </p>
                )}
              </div>
            </div>

            <DropdownMenu>
              <DropdownMenuTrigger asChild>
                <Button
                  size="icon"
                  variant="ghost"
                  className="absolute right-2 top-2 h-8 w-8 opacity-0 transition-opacity group-hover:opacity-100"
                  onClick={(e) => e.stopPropagation()}
                >
                  <MoreVertical className="h-4 w-4" />
                </Button>
              </DropdownMenuTrigger>
              <DropdownMenuContent
                align="end"
                className="border border-border bg-[hsl(var(--popover))] text-[hsl(var(--popover-foreground))] shadow-md"
              >
                {file.type === "file" && (
                  <>
                    <DropdownMenuItem onClick={() => onOpenFile?.(file)}>
                      <Eye className="mr-2 h-4 w-4" />
                      Preview
                    </DropdownMenuItem>
                    <DropdownMenuItem onClick={() => onDownloadFile?.(file)}>
                      <Download className="mr-2 h-4 w-4" />
                      Download
                    </DropdownMenuItem>
                    <DropdownMenuItem onClick={() => console.log("Edit", file.id)}>
                      <Edit className="mr-2 h-4 w-4" />
                      Edit
                    </DropdownMenuItem>
                  </>
                )}
                <DropdownMenuItem
                  onClick={() => onDeleteFile?.(file)}
                  className="text-destructive"
                >
                  <Trash2 className="mr-2 h-4 w-4" />
                  Delete
                </DropdownMenuItem>
              </DropdownMenuContent>
            </DropdownMenu>
          </Card>
        );
      })}
    </div>
  );
}
