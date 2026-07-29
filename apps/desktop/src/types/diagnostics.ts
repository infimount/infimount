export interface DiagnosticsSummary {
  appVersion: string;
  sidecarVersion: string | null;
  osArch: string;
  keyringStatus: string;
  configFileStatus: string;
  schemaVersions: Record<string, number>;
  storageCount: number;
  backendCounts: Record<string, number>;
  exposedStorageCount: number;
  enabledTools: string[];
  httpBindCategory: string;
  portAvailable: boolean;
  lastErrorCodes: string[];
  recentAuditDecisionCount: number;
}

export interface SanitizedErrorEntry {
  stage: string;
  errorCode: string;
  timestamp: string;
  count: number;
}

export interface RedactionManifest {
  redactedFields: string[];
  redactedCount: number;
}

export interface DiagnosticsBundle {
  summary: DiagnosticsSummary;
  sanitizedErrors: SanitizedErrorEntry[];
  redactionManifest: RedactionManifest;
  checksums: Record<string, string>;
}

export interface DiagnosticsExportResult {
  path: string;
  files: string[];
  checksums: Record<string, string>;
}

export type ProductEventName =
  | "app_launched"
  | "onboarding_started"
  | "onboarding_step_completed"
  | "storage_added"
  | "storage_validation_completed"
  | "workspace_created"
  | "sidecar_verified"
  | "client_config_previewed"
  | "client_config_applied"
  | "mcp_probe_completed"
  | "activation_completed";

export interface ProductEvent {
  id: string;
  timestamp: string;
  name: ProductEventName;
  schemaVersion: number;
  appVersion: string;
  osArch: string;
  backendType?: string;
  workspaceTemplate?: string;
  accessProfile?: string;
  clientKind?: string;
  success?: boolean;
  failureStage?: string;
  errorCode?: string;
  durationBucket?: string;
}

export interface OsInfo {
  osArch: string;
  appVersion: string;
}
