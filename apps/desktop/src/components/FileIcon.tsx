import { FileItem } from "@/types/storage";
import { cn } from "@/lib/utils";
import folderIcon from "@/assets/folder.svg";
import defaultIcon from "@/assets/file-icons/default.svg";

const iconModules = import.meta.glob("../assets/file-icons/*.svg", {
  eager: true,
  import: "default",
}) as Record<string, string>;

const iconByName = new Map<string, string>();
Object.entries(iconModules).forEach(([path, url]) => {
  const name = path.split("/").pop()?.replace(".svg", "").toLowerCase();
  if (name) {
    iconByName.set(name, url);
  }
});

export const getFileIconKey = (item: FileItem): string => {
  if (item.type === "folder") return "folder";

  const ext = item.extension?.toLowerCase();
  if (ext && iconByName.has(ext)) return ext;

  const name = item.name.toLowerCase();
  const stripped = name.startsWith(".") ? name.slice(1) : name;
  if (iconByName.has(stripped)) return stripped;

  return "default";
};

export const getFileIconPath = (item: FileItem): string => {
  const key = getFileIconKey(item);
  if (key === "folder") return folderIcon;
  return iconByName.get(key) ?? defaultIcon;
};

export const FileTypeIcon = ({
  item,
  className,
}: {
  item: FileItem;
  className?: string;
}) => (
  <img
    src={getFileIconPath(item)}
    alt=""
    aria-hidden="true"
    className={cn("inline-block", className)}
  />
);
