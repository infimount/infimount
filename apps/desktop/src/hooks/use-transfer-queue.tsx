/* eslint-disable react-refresh/only-export-components */

import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useState,
  type ReactNode,
} from "react";

import { listen } from "@tauri-apps/api/event";

import {
  cancelTransferJob,
  planTransferEntries,
  transferEntries,
  type TransferConflictPolicy,
  type TransferOperation,
  type TransferPlan,
} from "@/lib/api";
import { appendActivityLogEvent } from "@/lib/activityLog";

export type TransferJobStatus = "queued" | "running" | "completed" | "failed" | "cancelled";

export interface TransferJobRequest {
  fromSourceId: string;
  toSourceId: string;
  paths: string[];
  targetDir: string;
  operation: TransferOperation;
  conflictPolicy: TransferConflictPolicy;
  sourceName?: string;
  destinationName?: string;
}

export interface TransferJob extends TransferJobRequest {
  id: string;
  status: TransferJobStatus;
  progress: number;
  attempts: number;
  createdAt: number;
  updatedAt: number;
  error?: string;
  currentPath?: string;
  bytesTransferred?: number;
  totalBytes?: number;
  manifest?: TransferPlan;
  recoveryMode?: boolean;
}

interface TransferProgressPayload {
  jobId: string;
  completedItems: number;
  totalItems: number;
  bytesTransferred: number;
  totalBytes: number;
  currentPath: string;
}

interface TransferJobCallbacks {
  onCompleted?: (job: TransferJob) => void | Promise<void>;
  onFailed?: (job: TransferJob, error: unknown) => void | Promise<void>;
}

interface TransferQueueContextValue {
  jobs: TransferJob[];
  activeJob: TransferJob | null;
  enqueueTransfer: (request: TransferJobRequest, callbacks?: TransferJobCallbacks) => string;
  retryTransfer: (jobId: string) => void;
  cancelTransfer: (jobId: string) => void;
  clearCompletedTransfers: () => void;
  clearTransfer: (jobId: string) => void;
}

const TransferQueueContext = createContext<TransferQueueContextValue | null>(null);

const TRANSFER_HISTORY_STORAGE_KEY = "infimount:transfer-history:v1";
const MAX_PERSISTED_TRANSFERS = 50;
const TRANSFER_PROGRESS_PERSIST_DEBOUNCE_MS = 500;

const now = () => Date.now();

function createJob(request: TransferJobRequest): TransferJob {
  const timestamp = now();
  return {
    ...request,
    id: `transfer-${timestamp}-${Math.random().toString(36).slice(2, 9)}`,
    status: "queued",
    progress: 0,
    attempts: 0,
    createdAt: timestamp,
    updatedAt: timestamp,
  };
}

function normalizeError(error: unknown): string {
  if (error instanceof Error) return error.message;
  return String(error);
}

function isCancellationError(error: unknown) {
  return normalizeError(error).toLowerCase().includes("cancelled");
}

function isTransferJob(value: unknown): value is TransferJob {
  if (!value || typeof value !== "object") return false;
  const candidate = value as Partial<TransferJob>;
  return (
    typeof candidate.id === "string" &&
    typeof candidate.fromSourceId === "string" &&
    typeof candidate.toSourceId === "string" &&
    Array.isArray(candidate.paths) &&
    typeof candidate.targetDir === "string" &&
    (candidate.operation === "copy" || candidate.operation === "move") &&
    typeof candidate.conflictPolicy === "string" &&
    typeof candidate.progress === "number" &&
    typeof candidate.attempts === "number" &&
    typeof candidate.createdAt === "number" &&
    typeof candidate.updatedAt === "number"
  );
}

function readPersistedJobs(): TransferJob[] {
  if (typeof window === "undefined") return [];
  try {
    const parsed = JSON.parse(window.localStorage.getItem(TRANSFER_HISTORY_STORAGE_KEY) ?? "[]");
    if (!Array.isArray(parsed)) return [];
    return parsed.filter(isTransferJob).map((job) =>
      job.status === "queued" || job.status === "running"
        ? {
            ...job,
            status: "failed",
            progress: 100,
            error: "Interrupted before completion.",
            recoveryMode: true,
          }
        : job,
    );
  } catch {
    return [];
  }
}

function persistJobs(jobs: TransferJob[]) {
  if (typeof window === "undefined") return;
  const persisted = jobs
    .filter((job) => job.status !== "queued" || job.attempts > 0)
    .slice(-MAX_PERSISTED_TRANSFERS);
  window.localStorage.setItem(TRANSFER_HISTORY_STORAGE_KEY, JSON.stringify(persisted));
}

function progressPercent(payload: TransferProgressPayload) {
  if (payload.totalBytes > 0) {
    return Math.max(1, Math.min(99, Math.round((payload.bytesTransferred / payload.totalBytes) * 100)));
  }
  if (payload.totalItems > 0) {
    return Math.max(1, Math.min(99, Math.round((payload.completedItems / payload.totalItems) * 100)));
  }
  return 1;
}

export function TransferQueueProvider({ children }: { children: ReactNode }) {
  const [jobs, setJobs] = useState<TransferJob[]>(() => readPersistedJobs());
  const callbacksRef = useRef(new Map<string, TransferJobCallbacks>());
  const processingRef = useRef(false);
  const cancelledJobIdsRef = useRef(new Set<string>());
  const latestJobsRef = useRef(jobs);
  const persistTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  const patchJob = useCallback((jobId: string, patch: Partial<TransferJob>) => {
    setJobs((current) =>
      current.map((job) =>
        job.id === jobId
          ? {
              ...job,
              ...patch,
              updatedAt: now(),
            }
          : job,
      ),
    );
  }, []);

  const enqueueTransfer = useCallback(
    (request: TransferJobRequest, callbacks?: TransferJobCallbacks) => {
      const job = createJob(request);
      if (callbacks) callbacksRef.current.set(job.id, callbacks);
      setJobs((current) => [...current, job]);
      return job.id;
    },
    [],
  );

  const retryTransfer = useCallback((jobId: string) => {
    cancelledJobIdsRef.current.delete(jobId);
    setJobs((current) =>
      current.map((job) =>
        job.id === jobId && (job.status === "failed" || job.status === "cancelled")
          ? {
              ...job,
              status: "queued",
              progress: 0,
              conflictPolicy: job.recoveryMode && job.conflictPolicy === "fail" ? "skip" : job.conflictPolicy,
              error: undefined,
              updatedAt: now(),
            }
          : job,
      ),
    );
  }, []);

  const cancelTransfer = useCallback((jobId: string) => {
    cancelledJobIdsRef.current.add(jobId);
    setJobs((current) =>
      current.map((job) => {
        if (job.id !== jobId) return job;
        if (job.status === "queued") {
          return {
            ...job,
            status: "cancelled",
            progress: 0,
            updatedAt: now(),
          };
        }
        if (job.status === "running") {
          void cancelTransferJob(jobId);
          return {
            ...job,
            error: "Cancelling transfer...",
            updatedAt: now(),
          };
        }
        return job;
      }),
    );
  }, []);

  const clearCompletedTransfers = useCallback(() => {
    setJobs((current) =>
      current.filter((job) => job.status !== "completed" && job.status !== "cancelled"),
    );
  }, []);

  const clearTransfer = useCallback((jobId: string) => {
    callbacksRef.current.delete(jobId);
    setJobs((current) => current.filter((job) => job.id !== jobId || job.status === "running"));
  }, []);

  useEffect(() => {
    latestJobsRef.current = jobs;
    if (persistTimerRef.current) {
      clearTimeout(persistTimerRef.current);
      persistTimerRef.current = null;
    }

    const hasActiveTransfer = jobs.some((job) => job.status === "queued" || job.status === "running");
    if (!hasActiveTransfer) {
      persistJobs(jobs);
      return;
    }

    persistTimerRef.current = setTimeout(() => {
      persistJobs(latestJobsRef.current);
      persistTimerRef.current = null;
    }, TRANSFER_PROGRESS_PERSIST_DEBOUNCE_MS);
  }, [jobs]);

  useEffect(
    () => () => {
      if (persistTimerRef.current) {
        clearTimeout(persistTimerRef.current);
      }
      persistJobs(latestJobsRef.current);
    },
    [],
  );

  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let disposed = false;

    void listen<TransferProgressPayload>("infimount://transfer-progress", (event) => {
      if (disposed) return;
      const payload = event.payload;
      patchJob(payload.jobId, {
        progress: progressPercent(payload),
        currentPath: payload.currentPath || undefined,
        bytesTransferred: payload.bytesTransferred,
        totalBytes: payload.totalBytes,
      });
    })
      .then((cleanup) => {
        if (disposed) {
          cleanup();
        } else {
          unlisten = cleanup;
        }
      })
      .catch(() => {
        // Unit tests and non-Tauri previews do not provide the native event bus.
      });

    return () => {
      disposed = true;
      unlisten?.();
    };
  }, [patchJob]);

  useEffect(() => {
    if (processingRef.current) return;

    const nextJob = jobs.find((job) => job.status === "queued");
    if (!nextJob) return;

    processingRef.current = true;

    const run = async () => {
      patchJob(nextJob.id, {
        status: "running",
        progress: 12,
        attempts: nextJob.attempts + 1,
        currentPath: "Planning transfer...",
        error: undefined,
      });

      try {
        const effectiveConflictPolicy =
          nextJob.recoveryMode && nextJob.conflictPolicy === "fail" ? "skip" : nextJob.conflictPolicy;

        if (nextJob.recoveryMode) {
          appendActivityLogEvent({
            type: "transfer_recovery_started",
            jobId: nextJob.id,
            operation: nextJob.operation,
            sourceId: nextJob.fromSourceId,
            targetId: nextJob.toSourceId,
            pathCount: nextJob.paths.length,
            message: "Recovering interrupted transfer with completed-file skip behavior.",
          });
        }

        const manifest = await planTransferEntries(
          nextJob.fromSourceId,
          nextJob.toSourceId,
          nextJob.paths,
          nextJob.targetDir,
          nextJob.operation,
          effectiveConflictPolicy,
          nextJob.id,
        );
        if (cancelledJobIdsRef.current.has(nextJob.id)) {
          throw new Error("Transfer cancelled");
        }

        patchJob(nextJob.id, {
          progress: 20,
          manifest,
          conflictPolicy: effectiveConflictPolicy,
          currentPath: "Transfer plan ready.",
        });
        appendActivityLogEvent({
          type: "transfer_planned",
          jobId: nextJob.id,
          operation: nextJob.operation,
          sourceId: nextJob.fromSourceId,
          targetId: nextJob.toSourceId,
          pathCount: nextJob.paths.length,
          summary: manifest.summary as unknown as Record<string, unknown>,
        });
        patchJob(nextJob.id, { progress: 35, currentPath: "Starting transfer..." });
        appendActivityLogEvent({
          type: "transfer_started",
          jobId: nextJob.id,
          operation: nextJob.operation,
          sourceId: nextJob.fromSourceId,
          targetId: nextJob.toSourceId,
          pathCount: nextJob.paths.length,
        });
        await transferEntries(
          nextJob.fromSourceId,
          nextJob.toSourceId,
          nextJob.paths,
          nextJob.targetDir,
          nextJob.operation,
          effectiveConflictPolicy,
          nextJob.id,
        );
        const completedJob: TransferJob = {
          ...nextJob,
          status: "completed",
          progress: 100,
          attempts: nextJob.attempts + 1,
          conflictPolicy: effectiveConflictPolicy,
          manifest,
          recoveryMode: false,
          updatedAt: now(),
        };
        patchJob(nextJob.id, {
          status: "completed",
          progress: 100,
          conflictPolicy: effectiveConflictPolicy,
          manifest,
          recoveryMode: false,
          currentPath: undefined,
        });
        appendActivityLogEvent({
          type: "transfer_completed",
          jobId: nextJob.id,
          operation: nextJob.operation,
          sourceId: nextJob.fromSourceId,
          targetId: nextJob.toSourceId,
          pathCount: nextJob.paths.length,
          summary: manifest.summary as unknown as Record<string, unknown>,
        });
        await callbacksRef.current.get(nextJob.id)?.onCompleted?.(completedJob);
      } catch (error) {
        if (isCancellationError(error)) {
          patchJob(nextJob.id, {
            status: "cancelled",
            progress: 0,
            currentPath: undefined,
            error: undefined,
          });
          appendActivityLogEvent({
            type: "transfer_cancelled",
            jobId: nextJob.id,
            operation: nextJob.operation,
            sourceId: nextJob.fromSourceId,
            targetId: nextJob.toSourceId,
            pathCount: nextJob.paths.length,
          });
          return;
        }

        const failedJob: TransferJob = {
          ...nextJob,
          status: "failed",
          progress: 100,
          attempts: nextJob.attempts + 1,
          error: normalizeError(error),
          updatedAt: now(),
        };
        patchJob(nextJob.id, {
          status: "failed",
          progress: 100,
          currentPath: undefined,
          error: failedJob.error,
        });
        appendActivityLogEvent({
          type: "transfer_failed",
          jobId: nextJob.id,
          operation: nextJob.operation,
          sourceId: nextJob.fromSourceId,
          targetId: nextJob.toSourceId,
          pathCount: nextJob.paths.length,
          message: failedJob.error,
        });
        await callbacksRef.current.get(nextJob.id)?.onFailed?.(failedJob, error);
      } finally {
        processingRef.current = false;
        setJobs((current) => [...current]);
      }
    };

    void run();

  }, [jobs, patchJob]);

  const activeJob = jobs.find((job) => job.status === "running") ?? null;

  const value = useMemo<TransferQueueContextValue>(
    () => ({
      jobs,
      activeJob,
      enqueueTransfer,
      retryTransfer,
      cancelTransfer,
      clearCompletedTransfers,
      clearTransfer,
    }),
    [
      activeJob,
      cancelTransfer,
      clearCompletedTransfers,
      clearTransfer,
      enqueueTransfer,
      jobs,
      retryTransfer,
    ],
  );

  return <TransferQueueContext.Provider value={value}>{children}</TransferQueueContext.Provider>;
}

export function useTransferQueue() {
  const context = useContext(TransferQueueContext);
  if (!context) {
    throw new Error("useTransferQueue must be used within TransferQueueProvider");
  }
  return context;
}
