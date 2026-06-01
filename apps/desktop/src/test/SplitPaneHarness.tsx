import { useState } from "react";
import { PanelLeft, PanelRight, X } from "lucide-react";

import { FileBrowser, type FileBrowserPaneState } from "@/components/FileBrowser";
import { WindowControls } from "@/components/WindowControls";
import { Button } from "@/components/ui/button";
import { ResizableHandle, ResizablePanel, ResizablePanelGroup } from "@/components/ui/resizable";

export function SplitPaneHarness() {
  const [isSidebarOpen, setIsSidebarOpen] = useState(true);
  const [isDualPaneOpen, setIsDualPaneOpen] = useState(false);
  const [primaryPaneState, setPrimaryPaneState] = useState<FileBrowserPaneState | null>(null);
  const [secondaryPaneState, setSecondaryPaneState] = useState<FileBrowserPaneState | null>(null);

  if (!isDualPaneOpen) {
    return (
      <div className="h-full overflow-hidden rounded-[12px] border border-border/40 bg-background">
        <FileBrowser
          sourceId="local"
          storageName="Local Docs"
          onToggleSidebar={() => setIsSidebarOpen((current) => !current)}
          isSidebarOpen={isSidebarOpen}
          onToggleDualPane={() => {
            setSecondaryPaneState(null);
            setIsDualPaneOpen(true);
          }}
          onPaneStateChange={setPrimaryPaneState}
        />
      </div>
    );
  }

  const currentPath = primaryPaneState?.currentPath ?? "/";
  const rightPath = secondaryPaneState?.currentPath ?? currentPath;

  return (
    <div className="flex h-full overflow-hidden rounded-[12px] border border-border/40 bg-background">
      <div className="flex min-w-0 flex-1 flex-col overflow-hidden bg-background">
        <div className="flex h-12 shrink-0 items-center gap-2 border-b border-border/70 bg-muted/30 px-4" data-tauri-drag-region>
          <div className="flex items-center gap-1 tauri-no-drag">
            <Button
              size="icon"
              variant="ghost"
              className="h-8 w-8 text-foreground/70 hover:bg-black/5 dark:hover:bg-white/5"
              onClick={() => setIsSidebarOpen((current) => !current)}
              title={isSidebarOpen ? "Hide Storage Sidebar" : "Show Storage Sidebar"}
              aria-label={isSidebarOpen ? "Hide Storage Sidebar" : "Show Storage Sidebar"}
            >
              {isSidebarOpen ? <PanelRight className="h-4 w-4" /> : <PanelLeft className="h-4 w-4" />}
            </Button>
          </div>
          <div className="min-w-0 flex-1" data-tauri-drag-region>
            <div className="truncate text-sm font-medium text-foreground">Local Docs</div>
            <div className="truncate text-[11px] text-muted-foreground">
              Split view, two panes in the same storage
            </div>
          </div>
          <div className="flex items-center gap-2 tauri-no-drag">
            <Button
              size="sm"
              variant="ghost"
              className="h-8 gap-1.5 px-2 text-xs text-foreground/75 hover:bg-black/5 dark:hover:bg-white/5"
              onClick={() => setIsDualPaneOpen(false)}
              aria-label="Close split pane"
              title="Close split pane"
            >
              <X className="h-4 w-4" />
              Close Split
            </Button>
            <div className="ml-1 border-l border-border/50 pl-2">
              <WindowControls />
            </div>
          </div>
        </div>
        <ResizablePanelGroup direction="horizontal" className="min-h-0 flex-1">
          <ResizablePanel defaultSize={50} minSize={25} className="overflow-hidden">
            <FileBrowser
              sourceId="local"
              storageName="Local Docs"
              showWindowControls={false}
              showTransferQueue={false}
              isDualPane
              headerVariant="pane"
              paneLabel="Left"
              initialPath={currentPath}
              paneTransferTarget={{
                sourceId: "local",
                storageName: "Local Docs",
                currentPath: rightPath,
                direction: "right",
              }}
              onPaneStateChange={setPrimaryPaneState}
            />
          </ResizablePanel>
          <ResizableHandle className="flex w-px flex-col items-center justify-center bg-transparent group/handle relative z-10">
            <div className="absolute inset-y-0 -left-1 -right-1 z-50 cursor-col-resize" />
            <div className="h-full w-[1px] bg-border/50 transition-colors group-hover/handle:bg-primary/40" />
          </ResizableHandle>
          <ResizablePanel defaultSize={50} minSize={25} className="overflow-hidden">
            <div className="flex h-full flex-col border-l border-border/40 bg-background">
              <FileBrowser
                sourceId="local"
                storageName="Local Docs"
                showWindowControls={false}
                showTransferQueue={false}
                isDualPane
                headerVariant="pane"
                paneLabel="Right"
                initialPath={currentPath}
                paneTransferTarget={{
                  sourceId: "local",
                  storageName: "Local Docs",
                  currentPath,
                  direction: "left",
                }}
                onPaneStateChange={setSecondaryPaneState}
              />
            </div>
          </ResizablePanel>
        </ResizablePanelGroup>
      </div>
    </div>
  );
}
