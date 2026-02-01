import { FileItem } from "@/types/storage";
import { cn } from "@/lib/utils";
import { DEFAULT_ICON_THEME, type IconTheme, useIconTheme } from "@/hooks/use-icon-theme";

const classicModules = import.meta.glob("../assets/file-icons-classic/*.svg", {
  eager: true,
  import: "default",
}) as Record<string, string>;

const modernModules = import.meta.glob("../assets/file-icons-modern/*.svg", {
  eager: true,
  import: "default",
}) as Record<string, string>;

const vividModules = import.meta.glob("../assets/file-icons-vivid/*.svg", {
  eager: true,
  import: "default",
}) as Record<string, string>;

const squareModules = import.meta.glob("../assets/file-cons-square/*.svg", {
  eager: true,
  import: "default",
}) as Record<string, string>;

const buildIconMap = (modules: Record<string, string>) => {
  const map = new Map<string, string>();
  Object.entries(modules).forEach(([path, url]) => {
    const name = path.split("/").pop()?.replace(".svg", "").toLowerCase();
    if (name) {
      map.set(name, url);
    }
  });
  return map;
};

const iconMaps: Record<IconTheme, Map<string, string>> = {
  classic: buildIconMap(classicModules),
  modern: buildIconMap(modernModules),
  vivid: buildIconMap(vividModules),
  square: buildIconMap(squareModules),
};

const iconNameSet = new Set<string>();
(Object.keys(iconMaps) as IconTheme[]).forEach((theme) => {
  iconMaps[theme].forEach((_value, key) => iconNameSet.add(key));
});

const defaultThemeMap = iconMaps[DEFAULT_ICON_THEME];
const defaultFallbackIcon = defaultThemeMap.get("default") ?? "";

const getIconForTheme = (theme: IconTheme, key: string) => {
  const themeMap = iconMaps[theme] ?? defaultThemeMap;
  return themeMap.get(key)
    ?? themeMap.get("default")
    ?? defaultThemeMap.get(key)
    ?? defaultFallbackIcon;
};

export const getFileIconKey = (item: FileItem): string => {
  if (item.type === "folder") return "folder";

  const ext = item.extension?.toLowerCase();
  if (ext && iconNameSet.has(ext)) return ext;

  const name = item.name.toLowerCase();
  const stripped = name.startsWith(".") ? name.slice(1) : name;
  if (iconNameSet.has(stripped)) return stripped;

  return "default";
};

export const getFileIconPath = (
  item: FileItem,
  theme: IconTheme = DEFAULT_ICON_THEME,
): string => {
  const key = getFileIconKey(item);
  return getIconForTheme(theme, key);
};

export const FileTypeIcon = ({
  item,
  className,
}: {
  item: FileItem;
  className?: string;
}) => {
  const { theme } = useIconTheme();

  return (
    <img
      src={getFileIconPath(item, theme)}
      alt=""
      aria-hidden="true"
      className={cn("block object-contain object-center", className)}
    />
  );
};
