import { FileItem } from "@/types/storage";
import {
  File,
  Folder,
  FileText,
  FileImage,
  FileVideo,
  FileArchive,
  FileCode,
  FileSpreadsheet,
  Music,
  Package,
} from "lucide-react";

export const getFileIcon = (item: FileItem) => {
  if (item.type === "folder") return Folder;

  const ext = item.extension?.toLowerCase();
  if (!ext) return File;

  // Images
  if (["jpg", "jpeg", "png", "gif", "webp", "svg", "bmp", "ico"].includes(ext)) {
    return FileImage;
  }

  // Videos
  if (["mp4", "avi", "mov", "mkv", "webm", "flv", "wmv"].includes(ext)) {
    return FileVideo;
  }

  // Archives
  if (["zip", "rar", "7z", "tar", "gz", "bz2", "xz"].includes(ext)) {
    return FileArchive;
  }

  // Documents
  if (["txt", "md", "doc", "docx", "pdf", "rtf", "odt"].includes(ext)) {
    return FileText;
  }

  // Code files
  if (
    [
      "js",
      "ts",
      "jsx",
      "tsx",
      "py",
      "java",
      "cpp",
      "c",
      "html",
      "css",
      "json",
      "xml",
      "yaml",
      "yml",
      "sh",
      "bash",
    ].includes(ext)
  ) {
    return FileCode;
  }

  // Spreadsheets
  if (["xls", "xlsx", "csv", "ods"].includes(ext)) {
    return FileSpreadsheet;
  }

  // Audio
  if (["mp3", "wav", "ogg", "flac", "aac", "m4a"].includes(ext)) {
    return Music;
  }

  // Packages
  if (["deb", "rpm", "dmg", "pkg", "apk"].includes(ext)) {
    return Package;
  }

  return File;
};

export const getFileColor = (item: FileItem) => {
  if (item.type === "folder") return "text-amber-500 dark:text-amber-400";

  const ext = item.extension?.toLowerCase();
  if (!ext) return "text-muted-foreground";

  // Images - Pink
  if (["jpg", "jpeg", "png", "gif", "webp", "svg", "bmp", "ico"].includes(ext)) {
    return "text-pink-600 dark:text-pink-400";
  }

  // Videos - Purple
  if (["mp4", "avi", "mov", "mkv", "webm", "flv", "wmv"].includes(ext)) {
    return "text-purple-600 dark:text-purple-400";
  }

  // Archives - Orange
  if (["zip", "rar", "7z", "tar", "gz", "bz2", "xz"].includes(ext)) {
    return "text-orange-600 dark:text-orange-400";
  }

  // Documents - Blue
  if (["txt", "md", "doc", "docx", "pdf", "rtf", "odt"].includes(ext)) {
    return "text-blue-600 dark:text-blue-400";
  }

  // Code - Green
  if (
    [
      "js",
      "ts",
      "jsx",
      "tsx",
      "py",
      "java",
      "cpp",
      "c",
      "html",
      "css",
      "json",
      "xml",
      "yaml",
      "yml",
      "sh",
      "bash",
    ].includes(ext)
  ) {
    return "text-green-600 dark:text-green-400";
  }

  // Spreadsheets - Teal
  if (["xls", "xlsx", "csv", "ods"].includes(ext)) {
    return "text-teal-600 dark:text-teal-400";
  }

  // Audio - Cyan
  if (["mp3", "wav", "ogg", "flac", "aac", "m4a"].includes(ext)) {
    return "text-cyan-600 dark:text-cyan-400";
  }

  // Packages - Red
  if (["deb", "rpm", "dmg", "pkg", "apk"].includes(ext)) {
    return "text-red-600 dark:text-red-400";
  }

  return "text-muted-foreground";
};

