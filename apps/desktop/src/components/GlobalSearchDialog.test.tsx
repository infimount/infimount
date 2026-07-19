import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { GlobalSearchDialog } from "./GlobalSearchDialog";
import { listEntriesRecursive } from "@/lib/api";
import type { StorageConfig } from "@/types/storage";

vi.mock("@/lib/api", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/lib/api")>();
  return {
    ...actual,
    listEntriesRecursive: vi.fn(),
  };
});

vi.mock("@/hooks/use-toast", () => ({
  toast: vi.fn(),
  useToast: () => ({ toast: vi.fn() }),
}));

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (error: unknown) => void;
  const promise = new Promise<T>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

const policy: StorageConfig["mcpPolicy"] = {
  version: 2,
  default_access: "read_write",
  rules: [],
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
};

const storages: StorageConfig[] = [
  {
    id: "local",
    type: "local-fs",
    name: "Local Docs",
    backend: "local",
    config: { root: "/tmp/docs" },
    enabled: true,
    mcpExposed: true,
    readOnly: false,
    connected: true,
    createdAt: "2026-01-01T00:00:00Z",
    updatedAt: "2026-01-01T00:00:00Z",
    mcpPolicy: policy,
  },
];

describe("GlobalSearchDialog", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    window.localStorage.clear();
  });

  it("cancels indexing and ignores stale storage responses", async () => {
    const pending = deferred<Awaited<ReturnType<typeof listEntriesRecursive>>>();
    vi.mocked(listEntriesRecursive).mockReturnValueOnce(pending.promise);

    render(
      <GlobalSearchDialog
        open
        storages={storages}
        onOpenChange={vi.fn()}
        onSelectStorage={vi.fn()}
      />,
    );

    fireEvent.click(screen.getByRole("button", { name: "Index Local Docs" }));
    expect(await screen.findByRole("button", { name: "Stop" })).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Stop" }));

    pending.resolve([
      {
        path: "reports/stale.txt",
        name: "stale.txt",
        is_dir: false,
        size: 12,
        modified_at: null,
      },
    ]);

    await waitFor(() => {
      expect(screen.queryByText(/1 paths/)).not.toBeInTheDocument();
      expect(window.localStorage.getItem("infimount:storage-index:v1")).toBeNull();
    });
    expect(screen.getByRole("button", { name: "Index Local Docs" })).toBeEnabled();
  });

  it("ignores indexing responses after dialog unmount", async () => {
    const pending = deferred<Awaited<ReturnType<typeof listEntriesRecursive>>>();
    vi.mocked(listEntriesRecursive).mockReturnValueOnce(pending.promise);

    const { unmount } = render(
      <GlobalSearchDialog
        open
        storages={storages}
        onOpenChange={vi.fn()}
        onSelectStorage={vi.fn()}
      />,
    );

    fireEvent.click(screen.getByRole("button", { name: "Index Local Docs" }));
    expect(await screen.findByRole("button", { name: "Stop" })).toBeInTheDocument();
    unmount();

    pending.resolve([
      {
        path: "reports/closed.txt",
        name: "closed.txt",
        is_dir: false,
        size: 12,
        modified_at: null,
      },
    ]);

    await waitFor(() => {
      expect(window.localStorage.getItem("infimount:storage-index:v1")).toBeNull();
    });
  });

  it("indexes storage metadata locally and searches paths", async () => {
    vi.mocked(listEntriesRecursive).mockResolvedValueOnce([
      {
        path: "reports/quarterly.txt",
        name: "quarterly.txt",
        is_dir: false,
        size: 12,
        modified_at: null,
      },
    ]);
    const onSelectStorage = vi.fn();
    const onOpenChange = vi.fn();

    render(
      <GlobalSearchDialog
        open
        storages={storages}
        onOpenChange={onOpenChange}
        onSelectStorage={onSelectStorage}
      />,
    );

    fireEvent.click(screen.getByRole("button", { name: "Index Local Docs" }));
    await screen.findByText(/1 paths/);

    fireEvent.change(screen.getByPlaceholderText("Search indexed paths..."), {
      target: { value: "quarterly" },
    });

    expect(await screen.findByText("quarterly.txt")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: /quarterly.txt/ }));

    expect(onSelectStorage).toHaveBeenCalledWith("local");
    expect(onOpenChange).toHaveBeenCalledWith(false);
    await waitFor(() => {
      expect(window.localStorage.getItem("infimount:storage-index:v1")).toContain(
        "quarterly.txt",
      );
    });
  });
});
