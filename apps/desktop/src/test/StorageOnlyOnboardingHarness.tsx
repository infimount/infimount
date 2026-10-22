import { useState } from "react";

import { ActivationWizard } from "@/components/ActivationWizard";

export function StorageOnlyOnboardingHarness({ storagesCount = 1 }: { storagesCount?: number }) {
  const [browserReady, setBrowserReady] = useState(false);

  return (
    <>
      <div data-testid="storage-only-state">
        {browserReady ? "File browser ready" : "Onboarding"}
      </div>
      <ActivationWizard
        open={!browserReady}
        onOpenChange={() => undefined}
        onAddStorage={() => undefined}
        onCreateDemo={async () => undefined}
        onOpenWorkspaces={() => undefined}
        onOpenMcpSettings={() => undefined}
        onPrepareAgentAccess={async () => {
          throw new Error("Agent setup is not part of storage-only onboarding");
        }}
        onComplete={async () => {
          throw new Error("Agent activation is not part of storage-only onboarding");
        }}
        onBrowseStorageOnly={async () => setBrowserReady(true)}
        onSaveState={async () => undefined}
        storagesCount={storagesCount}
        workspaces={[]}
        agentAccessReady={false}
        initialStep="welcome"
        initialCompletedSteps={[]}
      />
    </>
  );
}
