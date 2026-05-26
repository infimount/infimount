import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeAll, describe, expect, it, vi } from "vitest";

import { AddStorageDialog } from "./AddStorageDialog";
import type { StorageConfig, StorageValidationResult } from "@/types/storage";
import type { StorageKindSchema } from "@/lib/api";

const schemas: StorageKindSchema[] = [
  {
    id: "local-fs",
    label: "Local Folder",
    kind: "local",
    fields: [{ name: "root", label: "Root path", input_type: "text", required: true }],
  },
  {
    id: "aws-s3",
    label: "Amazon S3",
    kind: "s3",
    fields: [
      { name: "bucket", label: "Bucket", input_type: "text", required: true },
      { name: "region", label: "Region", input_type: "text" },
      { name: "secret", label: "Secret key", input_type: "text", secret: true },
      { name: "notes", label: "Notes", input_type: "textarea" },
    ],
  },
];

const validResult: StorageValidationResult = {
  valid: true,
  details: "ok",
  capabilities: {
    list: true,
    stat: true,
    read: true,
    write: true,
    delete: false,
    copy: false,
    rename: false,
    presign_read: false,
    create_dir: true,
    write_with_user_metadata: false,
  },
};

const renderDialog = (props: Partial<Parameters<typeof AddStorageDialog>[0]> = {}) => {
  const onOpenChange = vi.fn();
  const loadSchemas = vi.fn().mockResolvedValue(schemas);

  render(
    <AddStorageDialog open onOpenChange={onOpenChange} loadSchemas={loadSchemas} {...props} />,
  );

  return { onOpenChange, loadSchemas };
};

beforeAll(() => {
  if (!HTMLElement.prototype.hasPointerCapture) {
    Object.defineProperty(HTMLElement.prototype, "hasPointerCapture", {
      value: () => false,
      configurable: true,
    });
  }
  if (!HTMLElement.prototype.setPointerCapture) {
    Object.defineProperty(HTMLElement.prototype, "setPointerCapture", {
      value: () => undefined,
      configurable: true,
    });
  }
  if (!HTMLElement.prototype.releasePointerCapture) {
    Object.defineProperty(HTMLElement.prototype, "releasePointerCapture", {
      value: () => undefined,
      configurable: true,
    });
  }
});

describe("AddStorageDialog", () => {
  it("validates and submits a local storage draft", async () => {
    const onAdd = vi.fn().mockResolvedValue(undefined);
    const onVerify = vi.fn().mockResolvedValue(validResult);
    const { onOpenChange } = renderDialog({ onAdd, onVerify });

    const rootInput = await screen.findByLabelText(/Root path/);
    fireEvent.change(screen.getByLabelText("Storage Name"), {
      target: { value: "Local docs" },
    });
    fireEvent.change(rootInput, {
      target: { value: "/Users/me/docs" },
    });

    fireEvent.click(screen.getByRole("button", { name: "Validate" }));
    expect(await screen.findByText("Storage validated successfully.")).toBeInTheDocument();
    expect(screen.getByText("list")).toBeInTheDocument();
    expect(onVerify).toHaveBeenCalledWith({
      name: "Local docs",
      backend: "local",
      config: { root: "/Users/me/docs" },
      enabled: true,
      mcpExposed: true,
      readOnly: false,
    });

    fireEvent.click(screen.getByRole("button", { name: "Add Storage" }));

    await waitFor(() => {
      expect(onAdd).toHaveBeenCalledWith({
        name: "Local docs",
        backend: "local",
        config: { root: "/Users/me/docs" },
        enabled: true,
        mcpExposed: true,
        readOnly: false,
      });
      expect(onOpenChange).toHaveBeenCalledWith(false);
    });
  });

  it("shows required field errors before validation", async () => {
    const onVerify = vi.fn();
    renderDialog({ onVerify });

    await screen.findByLabelText(/Root path/);
    const nameInput = screen.getByLabelText("Storage Name");
    fireEvent.change(nameInput, {
      target: { value: "Incomplete" },
    });
    expect(nameInput).toHaveValue("Incomplete");

    fireEvent.click(screen.getByRole("button", { name: "Validate" }));

    await waitFor(() => {
      expect(screen.getByText("Root path is required.")).toBeInTheDocument();
    });
    expect(onVerify).not.toHaveBeenCalled();
  });

  it("switches schemas, masks secrets, resets fields, and submits S3 config", async () => {
    const onAdd = vi.fn().mockResolvedValue(undefined);
    renderDialog({
      onAdd,
      loadSchemas: vi.fn().mockResolvedValue([schemas[1]]),
    });

    await screen.findByLabelText(/Bucket/);
    fireEvent.change(screen.getByLabelText("Storage Name"), {
      target: { value: "Artifacts" },
    });
    fireEvent.change(screen.getByLabelText(/Bucket/), {
      target: { value: "release-artifacts" },
    });
    fireEvent.change(screen.getByLabelText(/Region/), {
      target: { value: "us-east-1" },
    });
    const secretInput = screen.getByLabelText(/Secret key/) as HTMLInputElement;
    fireEvent.change(secretInput, { target: { value: "top-secret" } });
    expect(secretInput).toHaveAttribute("type", "text");

    fireEvent.click(screen.getByRole("button", { name: "Mask Secrets" }));
    expect(screen.getByLabelText(/Secret key/)).toHaveAttribute("type", "password");
    fireEvent.click(screen.getByRole("button", { name: "Reveal Secrets" }));
    expect(screen.getByLabelText(/Secret key/)).toHaveAttribute("type", "text");

    fireEvent.click(screen.getByRole("button", { name: "Reset Fields" }));
    expect(screen.getByLabelText(/Bucket/)).toHaveValue("");
    fireEvent.change(screen.getByLabelText(/Bucket/), {
      target: { value: "release-artifacts" },
    });
    fireEvent.change(screen.getByLabelText(/Secret key/), {
      target: { value: "top-secret" },
    });

    fireEvent.click(screen.getByRole("button", { name: "Add Storage" }));

    await waitFor(() => {
      expect(onAdd).toHaveBeenCalledWith({
        name: "Artifacts",
        backend: "s3",
        config: { bucket: "release-artifacts", secret: "top-secret" },
        enabled: true,
        mcpExposed: true,
        readOnly: false,
      });
    });
  });

  it("applies S3-compatible provider presets", async () => {
    const onAdd = vi.fn().mockResolvedValue(undefined);
    renderDialog({
      onAdd,
      loadSchemas: vi.fn().mockResolvedValue([schemas[1]]),
    });

    expect(await screen.findByText("Provider presets")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: /Cloudflare R2/ }));
    expect(screen.getByLabelText("Storage Name")).toHaveValue("Cloudflare R2");
    expect(screen.getByLabelText(/Region/)).toHaveValue("auto");

    fireEvent.change(screen.getByLabelText(/Bucket/), {
      target: { value: "datasets" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Add Storage" }));

    await waitFor(() => {
      expect(onAdd).toHaveBeenCalledWith({
        name: "Cloudflare R2",
        backend: "s3",
        config: {
          bucket: "datasets",
          region: "auto",
        },
        enabled: true,
        mcpExposed: true,
        readOnly: false,
      });
    });
  });

  it("submits native Backblaze B2 storage drafts", async () => {
    const onAdd = vi.fn().mockResolvedValue(undefined);
    renderDialog({
      onAdd,
      loadSchemas: vi.fn().mockResolvedValue([
        {
          id: "backblaze-b2",
          label: "Backblaze B2",
          kind: "b2",
          fields: [
            { name: "bucket", label: "Bucket", input_type: "text", required: true },
            { name: "bucketId", label: "Bucket ID", input_type: "text", required: true },
            {
              name: "applicationKeyId",
              label: "Application Key ID",
              input_type: "text",
              required: true,
              secret: true,
            },
            {
              name: "applicationKey",
              label: "Application Key",
              input_type: "password",
              required: true,
              secret: true,
            },
            { name: "rootPath", label: "Root Prefix", input_type: "text" },
          ],
        },
      ]),
    });

    const bucketInput = await screen.findByLabelText(/^Bucket \*/i);
    fireEvent.change(screen.getByLabelText("Storage Name"), {
      target: { value: "B2 Archive" },
    });
    fireEvent.change(bucketInput, {
      target: { value: "archive" },
    });
    fireEvent.change(screen.getByLabelText(/Bucket ID/), { target: { value: "bucket-id" } });
    fireEvent.change(screen.getByLabelText(/Application Key ID/), {
      target: { value: "key-id" },
    });
    fireEvent.change(screen.getByLabelText(/^Application Key \*/i), {
      target: { value: "secret-key" },
    });
    fireEvent.change(screen.getByLabelText(/Root Prefix/), {
      target: { value: "/team" },
    });

    fireEvent.click(screen.getByRole("button", { name: "Add Storage" }));

    await waitFor(() => {
      expect(onAdd).toHaveBeenCalledWith({
        name: "B2 Archive",
        backend: "b2",
        config: {
          bucket: "archive",
          bucketId: "bucket-id",
          applicationKeyId: "key-id",
          applicationKey: "secret-key",
          rootPath: "/team",
        },
        enabled: true,
        mcpExposed: true,
        readOnly: false,
      });
    });
  });

  it("submits WebDAV compatibility flags as booleans", async () => {
    const onAdd = vi.fn().mockResolvedValue(undefined);
    renderDialog({
      onAdd,
      loadSchemas: vi.fn().mockResolvedValue([
        {
          id: "webdav",
          label: "WebDAV",
          kind: "webdav",
          fields: [
            { name: "serverUrl", label: "Server URL", input_type: "text", required: true },
            {
              name: "disableCreateDir",
              label: "Disable directory creation compatibility mode",
              input_type: "checkbox",
            },
          ],
        },
      ]),
    });

    const serverUrlInput = await screen.findByLabelText(/Server URL/);
    fireEvent.change(screen.getByLabelText("Storage Name"), {
      target: { value: "Strict WebDAV" },
    });
    fireEvent.change(serverUrlInput, {
      target: { value: "https://dav.example.test" },
    });
    fireEvent.click(screen.getByRole("switch", { name: /Disable directory creation/ }));
    fireEvent.click(screen.getByRole("button", { name: "Add Storage" }));

    await waitFor(() => {
      expect(onAdd).toHaveBeenCalledWith({
        name: "Strict WebDAV",
        backend: "webdav",
        config: {
          serverUrl: "https://dav.example.test",
          disableCreateDir: true,
        },
        enabled: true,
        mcpExposed: true,
        readOnly: false,
      });
    });
  });

  it("edits existing storage while preserving advanced config fields", async () => {
    const onUpdate = vi.fn().mockResolvedValue(undefined);
    const initialStorage: StorageConfig = {
      id: "store-1",
      type: "aws-s3",
      name: "Existing bucket",
      backend: "s3",
      config: {
        bucket: "old-bucket",
        secret: "saved-secret",
        endpoint: "https://s3.example.test",
      },
      enabled: false,
      mcpExposed: false,
      readOnly: true,
      connected: true,
      createdAt: "2026-01-01T00:00:00Z",
      updatedAt: "2026-01-02T00:00:00Z",
      mcpPolicy: {
        default_access: "read_only",
        allowed_paths: [],
        denied_paths: [],
        confirmation_rules: {
          require_for_write: true,
          require_for_overwrite: true,
          require_for_delete: true,
          require_for_version_delete: true,
          require_for_presign: true,
          require_for_cross_storage_copy: true,
        },
      },
    };

    renderDialog({ initialStorage, onUpdate });

    expect(await screen.findByText("Edit Storage")).toBeInTheDocument();
    await waitFor(() =>
      expect(screen.getByLabelText("Storage Name")).toHaveValue("Existing bucket"),
    );
    expect(await screen.findByLabelText(/Bucket/)).toHaveValue("old-bucket");
    expect(screen.getByLabelText(/Secret key/)).toHaveAttribute("type", "password");
    expect(screen.getByText(/advanced config field/)).toBeInTheDocument();

    fireEvent.change(screen.getByLabelText(/Bucket/), {
      target: { value: "new-bucket" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Save Changes" }));

    await waitFor(() => {
      expect(onUpdate).toHaveBeenCalledWith("store-1", {
        name: "Existing bucket",
        backend: "s3",
        config: {
          endpoint: "https://s3.example.test",
          bucket: "new-bucket",
          secret: "saved-secret",
        },
        enabled: false,
        mcpExposed: false,
        readOnly: true,
      });
    });
  });
});
