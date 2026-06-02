import { FileBrowser } from "@/components/FileBrowser";
import { AppZoomProvider } from "@/hooks/use-app-zoom";
import { FileClipboardProvider } from "@/hooks/use-file-clipboard";
import { TransferQueueProvider } from "@/hooks/use-transfer-queue";

export function DeleteProgressHarness() {
  return (
    <AppZoomProvider>
      <FileClipboardProvider>
        <TransferQueueProvider>
          <div className="h-full overflow-hidden rounded-[12px] border border-border/40 bg-background">
            <FileBrowser sourceId="local" storageName="Local Docs" showWindowControls={false} />
          </div>
        </TransferQueueProvider>
      </FileClipboardProvider>
    </AppZoomProvider>
  );
}
