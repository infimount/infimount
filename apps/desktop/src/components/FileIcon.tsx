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

  if (["jpg", "jpeg", "png", "gif", "webp", "svg", "bmp", "ico"].includes(ext)) {
    return FileImage;
  }

  if (["mp4", "avi", "mov", "mkv", "webm", "flv", "wmv"].includes(ext)) {
    return FileVideo;
  }

  if (["zip", "rar", "7z", "tar", "gz", "bz2", "xz"].includes(ext)) {
    return FileArchive;
  }

  if (["txt", "md", "doc", "docx", "pdf", "rtf", "odt"].includes(ext)) {
    return FileText;
  }

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

  if (["xls", "xlsx", "csv", "ods"].includes(ext)) {
    return FileSpreadsheet;
  }

  if (["mp3", "wav", "ogg", "flac", "aac", "m4a"].includes(ext)) {
    return Music;
  }

  if (["deb", "rpm", "dmg", "pkg", "apk"].includes(ext)) {
    return Package;
  }

  return File;
};

export const getFileColor = (item: FileItem) => {
  if (item.type === "folder") return "text-primary";

  const ext = item.extension?.toLowerCase();
  if (!ext) return "text-muted-foreground";

  if (["jpg", "jpeg", "png", "gif", "webp", "svg", "bmp", "ico"].includes(ext)) {
    return "text-pink-500";
  }

  if (["mp4", "avi", "mov", "mkv", "webm", "flv", "wmv"].includes(ext)) {
    return "text-purple-500";
  }

  if (["zip", "rar", "7z", "tar", "gz", "bz2", "xz"].includes(ext)) {
    return "text-amber-500";
  }

  if (["txt", "md", "doc", "docx", "pdf", "rtf", "odt"].includes(ext)) {
    return "text-blue-500";
  }

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
    return "text-emerald-500";
  }

  if (["xls", "xlsx", "csv", "ods"].includes(ext)) {
    return "text-teal-500";
  }

  if (["mp3", "wav", "ogg", "flac", "aac", "m4a"].includes(ext)) {
    return "text-cyan-500";
  }

  if (["deb", "rpm", "dmg", "pkg", "apk"].includes(ext)) {
    return "text-rose-500";
  }

  return "text-muted-foreground";
};

