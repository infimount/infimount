import { FilePreviewPanel } from "@/components/FilePreviewPanel";
import type { FileItem } from "@/types/storage";

const readFilePayload = Array.from(
  new TextEncoder().encode("# Agents\n\nInfimount preview content.\n\n- list_dir\n- read_file\n"),
);

const file: FileItem = {
  id: "/docs/agents.md",
  name: "agents.md",
  type: "file",
  size: 1280,
  modified: new Date("2026-03-19T07:00:00Z"),
  extension: "md",
};

export function MockedFilePreviewPanel() {
  Object.defineProperty(window, "__TAURI_INTERNALS__", {
    configurable: true,
    value: {
      invoke: async (cmd: string) => {
        if (cmd === "read_file") {
          return readFilePayload;
        }
        if (cmd === "stat_entry") {
          return {
            path: "/docs/agents.md",
            name: "agents.md",
            is_dir: false,
            size: readFilePayload.length,
            modified_at: "2026-03-19T07:00:00Z",
          };
        }
        return null;
      },
      transformCallback: (() => {
        let nextId = 1;
        return () => nextId++;
      })(),
      unregisterCallback: () => undefined,
    },
  });

  return (
    <FilePreviewPanel
      file={file}
      sourceId="demo-source"
      onClose={() => undefined}
      onDownload={() => undefined}
    />
  );
}
