import { FileItem } from "@/types/storage";
import { cn } from "@/lib/utils";
import { DEFAULT_ICON_THEME, type IconTheme, useIconTheme } from "@/hooks/use-icon-theme";
import { ICON_KEYS } from "./file-icon-themes/icon-keys";
import { useEffect, useState } from "react";

const EMPTY_ICON =
  "data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 24 24'%3E%3C/svg%3E";

type IconThemeMap = Map<string, string>;

const themeLoaders: Record<IconTheme, () => Promise<{ default: IconThemeMap }>> = {
  classic: () => import("./file-icon-themes/classic"),
  modern: () => import("./file-icon-themes/modern"),
  vivid: () => import("./file-icon-themes/vivid"),
  square: () => import("./file-icon-themes/square"),
};

const themeCache = new Map<IconTheme, IconThemeMap | Promise<IconThemeMap>>();
const iconUrlCache = new Map<string, string>();

const loadThemeMap = async (theme: IconTheme): Promise<IconThemeMap> => {
  const cached = themeCache.get(theme);
  if (cached instanceof Map) return cached;
  if (cached) return cached;

  const loader = themeLoaders[theme] ?? themeLoaders[DEFAULT_ICON_THEME];
  const promise = loader().then((module) => {
    themeCache.set(theme, module.default);
    return module.default;
  });
  themeCache.set(theme, promise);
  return promise;
};

export const getFileIconKey = (item: FileItem): string => {
  if (item.type === "folder") return "folder";

  const ext = item.extension?.toLowerCase();
  if (ext && ICON_KEYS.has(ext)) return ext;

  const name = item.name.toLowerCase();
  const stripped = name.startsWith(".") ? name.slice(1) : name;
  if (ICON_KEYS.has(stripped)) return stripped;

  return "default";
};

const getFileIconPathByKey = async (
  key: string,
  theme: IconTheme = DEFAULT_ICON_THEME,
): Promise<string> => {
  const cacheKey = `${theme}:${key}`;
  const cached = iconUrlCache.get(cacheKey);
  if (cached) return cached;

  const themeMap = await loadThemeMap(theme);
  let url = themeMap.get(key) ?? themeMap.get("default");
  if (!url && theme !== DEFAULT_ICON_THEME) {
    const defaultMap = await loadThemeMap(DEFAULT_ICON_THEME);
    url = defaultMap.get(key) ?? defaultMap.get("default");
  }
  url ??= EMPTY_ICON;
  iconUrlCache.set(cacheKey, url);
  return url;
};

export const getFileIconPath = async (
  item: FileItem,
  theme: IconTheme = DEFAULT_ICON_THEME,
): Promise<string> => getFileIconPathByKey(getFileIconKey(item), theme);

const useFileIconPath = (item: FileItem, theme: IconTheme) => {
  const key = getFileIconKey(item);
  const cacheKey = `${theme}:${key}`;
  const [loadedIcon, setLoadedIcon] = useState<{ cacheKey: string; path: string }>(() => ({
    cacheKey: "",
    path: EMPTY_ICON,
  }));

  useEffect(() => {
    let active = true;
    void getFileIconPathByKey(key, theme).then((nextPath) => {
      if (!active) return;
      setLoadedIcon({ cacheKey, path: nextPath });
    });
    return () => {
      active = false;
    };
  }, [cacheKey, key, theme]);

  return loadedIcon.cacheKey === cacheKey ? loadedIcon.path : EMPTY_ICON;
};

export const FileTypeIcon = ({ item, className }: { item: FileItem; className?: string }) => {
  const { theme } = useIconTheme();
  const iconPath = useFileIconPath(item, theme);

  return (
    <img
      src={iconPath}
      alt=""
      aria-hidden="true"
      draggable={false}
      className={cn("block object-contain object-center", className)}
    />
  );
};
