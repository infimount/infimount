import { expect, test } from "@playwright/experimental-ct-react";
import { AgentTaskPublicationPanel } from "@/components/AgentTaskPublicationPanel";
import type { AgentTaskOutputReviewOutput } from "@/lib/agentTasks";

const review: AgentTaskOutputReviewOutput = {
  workspaceId: "workspace-1",
  workspaceName: "Agent Scratch",
  taskId: "task-1",
  taskRoot: "tasks/task-1",
  fileCount: 2,
  totalBytes: 30,
  files: [
    {
      taskPath: "outputs/summary.md",
      byteSize: 12,
      sha256: "a".repeat(64),
      preview: "summary",
      previewUnavailableReason: null,
    },
    {
      taskPath: "outputs/data.csv",
      byteSize: 18,
      sha256: "b".repeat(64),
      preview: "a,b",
      previewUnavailableReason: null,
    },
  ],
};

test("reviews an exact Agent Task publication before create-only publish", async ({ mount, page }) => {
  const installMocks = () => {
    Object.defineProperty(window, "__TAURI_INTERNALS__", {
      configurable: true,
      value: {
        invoke: async (cmd: string) => {
          if (cmd === "list_storages") {
            return [
              {
                id: "destination-1",
                name: "Published",
                backend: "local",
                type: "local-fs",
                config: {},
                enabled: true,
                readOnly: false,
                connected: true,
                createdAt: "2026-09-08T00:00:00Z",
                updatedAt: "2026-09-08T00:00:00Z",
              },
            ];
          }
          if (cmd === "preview_agent_task_publication") {
            return {
              workspaceId: "workspace-1",
              workspaceName: "Agent Scratch",
              taskId: "task-1",
              taskRoot: "tasks/task-1",
              destinationStorageId: "destination-1",
              destinationStorageName: "Published",
              destinationDir: "exports",
              conflictPolicy: "fail",
              fileCount: 1,
              totalBytes: 12,
              createCount: 1,
              renameCount: 0,
              conflictCount: 0,
              canPublish: true,
              previewToken: "c".repeat(64),
              files: [
                {
                  taskPath: "outputs/summary.md",
                  byteSize: 12,
                  sha256: "a".repeat(64),
                  destinationPath: "exports/summary.md",
                  action: "create",
                },
              ],
            };
          }
          if (cmd === "publish_agent_task_outputs") {
            return {
              publicationId: "publication-1",
              publishedAt: "2026-09-08T01:00:00Z",
              workspaceId: "workspace-1",
              taskId: "task-1",
              destinationStorageId: "destination-1",
              destinationStorageName: "Published",
              receiptPath: "tasks/task-1/publish-receipt-publication-1.json",
              files: [
                {
                  taskPath: "outputs/summary.md",
                  byteSize: 12,
                  sha256: "a".repeat(64),
                  destinationPath: "exports/summary.md",
                  action: "create",
                },
              ],
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
  };

  await page.addInitScript(installMocks);
  await page.evaluate(installMocks);

  await mount(
    <div className="min-h-screen bg-background p-8 text-foreground">
      <AgentTaskPublicationPanel review={review} />
    </div>,
  );

  const panel = page.getByTestId("agent-task-publication");
  await expect(panel).toContainText("0 of 2 selected");
  await expect(page.getByRole("button", { name: "Review publication" })).toBeDisabled();
  await expect(page.getByRole("button", { name: /^Publish / })).toHaveCount(0);

  await page.getByRole("checkbox", { name: "Publish outputs/summary.md" }).click();
  await page.getByLabel("Destination folder").fill("exports");
  await page.getByRole("button", { name: "Review publication" }).click();

  const preview = page.getByTestId("agent-task-publication-preview");
  await expect(preview).toContainText("exports/summary.md");
  await expect(preview).toContainText("Create 1");
  await expect(page.getByText(/^Overwrite$/)).toHaveCount(0);
  await expect(page.getByRole("button", { name: "Publish 1 approved output" })).toBeEnabled();

  await expect(panel).toHaveScreenshot("agent-task-publication-reviewed.png");

  await page.getByRole("button", { name: "Publish 1 approved output" }).click();
  await expect(page.getByTestId("agent-task-publication-success")).toContainText(
    "publish-receipt-publication-1.json",
  );
  await expect(page.getByRole("button", { name: /^Publish / })).toHaveCount(0);
  await expect(page.getByRole("button", { name: "Review publication" })).toBeEnabled();
});
