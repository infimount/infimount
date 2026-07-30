import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { RecoveryBackupDialog } from "./RecoveryBackupDialog";
import { createRecoveryBackup, previewRecoveryRestore } from "@/lib/api";

vi.mock("@/lib/api", () => ({
  createRecoveryBackup: vi.fn(),
  previewRecoveryRestore: vi.fn(),
  applyRecoveryRestore: vi.fn(),
}));

vi.mock("@/hooks/use-toast", () => ({
  useToast: () => ({ toast: vi.fn() }),
}));

describe("RecoveryBackupDialog secret handling", () => {
  afterEach(cleanup);

  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("clears create passphrases as soon as encryption completes", async () => {
    vi.mocked(createRecoveryBackup).mockResolvedValue({
      armored: "encrypted",
      storageCount: 1,
      hasNativeSecrets: true,
    });
    render(<RecoveryBackupDialog open onOpenChange={vi.fn()} />);

    fireEvent.change(screen.getByLabelText("Encryption passphrase"), {
      target: { value: "strong-passphrase" },
    });
    fireEvent.change(screen.getByLabelText("Confirm passphrase"), {
      target: { value: "strong-passphrase" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Create Backup" }));

    await screen.findByText("Backup created successfully");
    expect(createRecoveryBackup).toHaveBeenCalledWith({ passphrase: "strong-passphrase" });
  });

  it("clears the restore passphrase immediately after preview", async () => {
    vi.mocked(previewRecoveryRestore).mockResolvedValue({
      previewId: "preview-1",
      storageCount: 0,
      storageAdditions: 0,
      storageUpdates: 0,
      storageRemovals: 0,
      hasMcpSettings: false,
      hasAppSettings: false,
      hasWorkspaces: false,
      hasSecrets: false,
      createdAt: "2026-01-01T00:00:00Z",
      checksumValid: true,
      unsupportedVersion: false,
      expiresInSeconds: 300,
    });
    render(<RecoveryBackupDialog open onOpenChange={vi.fn()} />);

    fireEvent.mouseDown(screen.getByRole("tab", { name: /Restore/ }), {
      button: 0,
      ctrlKey: false,
    });
    fireEvent.change(screen.getByLabelText("Armored backup content"), {
      target: { value: "-----BEGIN AGE ENCRYPTED FILE-----" },
    });
    const passphrase = screen.getByLabelText("Decryption passphrase");
    fireEvent.change(passphrase, { target: { value: "restore-passphrase" } });
    fireEvent.click(screen.getByRole("button", { name: "Preview Restore" }));

    await screen.findByText("Backup contents");
    await waitFor(() => expect(passphrase).toHaveValue(""));
  });
});
