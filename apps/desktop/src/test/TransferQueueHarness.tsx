import { TransferQueuePanel } from "@/components/TransferQueuePanel";
import { Button } from "@/components/ui/button";
import { TransferQueueProvider, useTransferQueue } from "@/hooks/use-transfer-queue";

function TransferQueueControls() {
  const { enqueueTransfer } = useTransferQueue();

  return (
    <div className="relative h-full w-full bg-background p-4">
      <Button
        type="button"
        onClick={() =>
          enqueueTransfer({
            fromSourceId: "local",
            toSourceId: "archive",
            sourceName: "Local Docs",
            destinationName: "Archive Bucket",
            paths: ["/report.txt"],
            targetDir: "/incoming",
            operation: "copy",
            conflictPolicy: "fail",
          })
        }
      >
        Queue copy
      </Button>
      <TransferQueuePanel />
    </div>
  );
}

export function TransferQueueHarness() {
  return (
    <TransferQueueProvider>
      <TransferQueueControls />
    </TransferQueueProvider>
  );
}
