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

import {
  transferEntries,
  type TransferConflictPolicy,
  type TransferOperation,
} from "@/lib/api";

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

export function TransferQueueProvider({ children }: { children: ReactNode }) {
  const [jobs, setJobs] = useState<TransferJob[]>([]);
  const callbacksRef = useRef(new Map<string, TransferJobCallbacks>());
  const processingRef = useRef(false);

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
    setJobs((current) =>
      current.map((job) =>
        job.id === jobId && (job.status === "failed" || job.status === "cancelled")
          ? {
              ...job,
              status: "queued",
              progress: 0,
              error: undefined,
              updatedAt: now(),
            }
          : job,
      ),
    );
  }, []);

  const cancelTransfer = useCallback((jobId: string) => {
    setJobs((current) =>
      current.map((job) =>
        job.id === jobId && job.status === "queued"
          ? {
              ...job,
              status: "cancelled",
              progress: 0,
              updatedAt: now(),
            }
          : job,
      ),
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
    if (processingRef.current) return;

    const nextJob = jobs.find((job) => job.status === "queued");
    if (!nextJob) return;

    processingRef.current = true;

    const run = async () => {
      patchJob(nextJob.id, {
        status: "running",
        progress: 12,
        attempts: nextJob.attempts + 1,
        error: undefined,
      });

      try {
        patchJob(nextJob.id, { progress: 35 });
        await transferEntries(
          nextJob.fromSourceId,
          nextJob.toSourceId,
          nextJob.paths,
          nextJob.targetDir,
          nextJob.operation,
          nextJob.conflictPolicy,
        );
        const completedJob: TransferJob = {
          ...nextJob,
          status: "completed",
          progress: 100,
          attempts: nextJob.attempts + 1,
          updatedAt: now(),
        };
        patchJob(nextJob.id, { status: "completed", progress: 100 });
        await callbacksRef.current.get(nextJob.id)?.onCompleted?.(completedJob);
      } catch (error) {
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
          error: failedJob.error,
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
