import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { RecoveryBackupDialog } from "./RecoveryBackupDialog";
import { applyRecoveryRestore, createRecoveryBackup, previewRecoveryRestore } from "@/lib/api";

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

  it("rejects short and mismatched create passphrases before calling the backend", async () => {
    render(<RecoveryBackupDialog open onOpenChange={vi.fn()} />);
    fireEvent.change(screen.getByLabelText("Encryption passphrase"), { target: { value: "short" } });
    fireEvent.change(screen.getByLabelText("Confirm passphrase"), { target: { value: "short" } });
    fireEvent.click(screen.getByRole("button", { name: "Create Backup" }));
    expect(createRecoveryBackup).not.toHaveBeenCalled();

    fireEvent.change(screen.getByLabelText("Encryption passphrase"), { target: { value: "long-enough" } });
    fireEvent.change(screen.getByLabelText("Confirm passphrase"), { target: { value: "different" } });
    fireEvent.click(screen.getByRole("button", { name: "Create Backup" }));
    expect(createRecoveryBackup).not.toHaveBeenCalled();
  });

  it("clears create secrets when encryption fails", async () => {
    vi.mocked(createRecoveryBackup).mockRejectedValue(new Error("encryption unavailable"));
    render(<RecoveryBackupDialog open onOpenChange={vi.fn()} />);
    const passphrase = screen.getByLabelText("Encryption passphrase");
    fireEvent.change(passphrase, { target: { value: "strong-passphrase" } });
    fireEvent.change(screen.getByLabelText("Confirm passphrase"), { target: { value: "strong-passphrase" } });
    fireEvent.click(screen.getByRole("button", { name: "Create Backup" }));
    await waitFor(() => expect(passphrase).toHaveValue(""));
    expect(screen.queryByText("Backup created successfully")).not.toBeInTheDocument();
  });

  it("shows checksum and preview errors without retaining the restore secret", async () => {
    vi.mocked(previewRecoveryRestore).mockResolvedValueOnce({
      previewId: "bad", storageCount: 0, storageAdditions: 0, storageUpdates: 0, storageRemovals: 0,
      hasMcpSettings: false, hasAppSettings: false, hasWorkspaces: false, hasSecrets: false,
      createdAt: "2026-01-01T00:00:00Z", checksumValid: false, unsupportedVersion: false, expiresInSeconds: 300,
    }).mockRejectedValueOnce(new Error("invalid backup"));
    render(<RecoveryBackupDialog open onOpenChange={vi.fn()} />);
    fireEvent.mouseDown(screen.getByRole("tab", { name: /Restore/ }), { button: 0, ctrlKey: false });
    const armored = screen.getByLabelText("Armored backup content");
    const passphrase = screen.getByLabelText("Decryption passphrase");
    fireEvent.change(armored, { target: { value: "backup" } });
    fireEvent.change(passphrase, { target: { value: "restore-passphrase" } });
    fireEvent.click(screen.getByRole("button", { name: "Preview Restore" }));
    await screen.findByText("Backup checksum mismatch; data may be corrupted.");
    await waitFor(() => expect(passphrase).toHaveValue(""));

    fireEvent.change(passphrase, { target: { value: "restore-passphrase" } });
    fireEvent.click(screen.getByRole("button", { name: "Preview Restore" }));
    await screen.findByText("invalid backup");
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

  it("applies a reviewed restore and handles an apply failure", async () => {
    vi.mocked(previewRecoveryRestore).mockResolvedValue({
      previewId: "preview-apply", storageCount: 2, storageAdditions: 1, storageUpdates: 1, storageRemovals: 0,
      hasMcpSettings: true, hasAppSettings: true, hasWorkspaces: true, hasSecrets: true,
      createdAt: "2026-01-01T00:00:00Z", checksumValid: true, unsupportedVersion: true, expiresInSeconds: 61,
    });
    vi.mocked(applyRecoveryRestore).mockResolvedValue({
      storagesRestored: 2, mcpSettingsRestored: true, appSettingsRestored: true,
      workspacesRestored: true, secretsRestored: 1,
    });
    const onRestoreComplete = vi.fn();
    const onOpenChange = vi.fn();
    render(<RecoveryBackupDialog open onOpenChange={onOpenChange} onRestoreComplete={onRestoreComplete} />);
    fireEvent.mouseDown(screen.getByRole("tab", { name: /Restore/ }), { button: 0, ctrlKey: false });
    fireEvent.change(screen.getByLabelText("Armored backup content"), { target: { value: "backup" } });
    fireEvent.change(screen.getByLabelText("Decryption passphrase"), { target: { value: "restore-passphrase" } });
    fireEvent.click(screen.getByRole("button", { name: "Preview Restore" }));
    await screen.findByText("Unsupported backup format version");
    expect(screen.getByText(/Includes MCP settings/)).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Apply Restore" }));
    await waitFor(() => expect(applyRecoveryRestore).toHaveBeenCalledWith({
      previewId: "preview-apply", restoreMcpSettings: true, restoreAppSettings: true,
      restoreWorkspaces: true, restoreSecrets: true,
    }));
    expect(onRestoreComplete).toHaveBeenCalled();
    expect(onOpenChange).toHaveBeenCalledWith(false);

    vi.clearAllMocks();
    vi.mocked(previewRecoveryRestore).mockResolvedValue({
      previewId: "preview-fail", storageCount: 0, storageAdditions: 0, storageUpdates: 0, storageRemovals: 0,
      hasMcpSettings: false, hasAppSettings: false, hasWorkspaces: false, hasSecrets: false,
      createdAt: "2026-01-01T00:00:00Z", checksumValid: true, unsupportedVersion: false, expiresInSeconds: 300,
    });
    vi.mocked(applyRecoveryRestore).mockRejectedValue(new Error("restore failed"));
    cleanup();
    render(<RecoveryBackupDialog open onOpenChange={vi.fn()} />);
    fireEvent.mouseDown(screen.getByRole("tab", { name: /Restore/ }), { button: 0, ctrlKey: false });
    fireEvent.change(screen.getByLabelText("Armored backup content"), { target: { value: "backup" } });
    fireEvent.change(screen.getByLabelText("Decryption passphrase"), { target: { value: "restore-passphrase" } });
    fireEvent.click(screen.getByRole("button", { name: "Preview Restore" }));
    await screen.findByText("Backup contents");
    fireEvent.click(screen.getByRole("button", { name: "Apply Restore" }));
    await screen.findByText("restore failed");
  });
});
