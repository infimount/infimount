import * as React from "react";

export type FileClipboardOperation = "copy" | "move";

export type FileClipboardPayload = {
  operation: FileClipboardOperation;
  sourceId: string;
  paths: string[];
} | null;

type FileClipboardContextValue = {
  clipboard: FileClipboardPayload;
  setClipboard: (payload: FileClipboardPayload) => void;
  clearClipboard: () => void;
};

const FileClipboardContext = React.createContext<FileClipboardContextValue>({
  clipboard: null,
  setClipboard: () => {},
  clearClipboard: () => {},
});

export function FileClipboardProvider({ children }: { children: React.ReactNode }) {
  const [clipboard, setClipboard] = React.useState<FileClipboardPayload>(null);

  const clearClipboard = React.useCallback(() => setClipboard(null), []);

  const value = React.useMemo(
    () => ({
      clipboard,
      setClipboard,
      clearClipboard,
    }),
    [clipboard, clearClipboard],
  );

  return <FileClipboardContext.Provider value={value}>{children}</FileClipboardContext.Provider>;
}

export function useFileClipboard() {
  return React.useContext(FileClipboardContext);
}

