export type StorageType =
  | "aws-s3"
  | "backblaze-b2"
  | "aliyun-oss"
  | "tencent-cos"
  | "huawei-obs"
  | "azure-blob"
  | "webdav"
  | "gcs"
  | "google-drive"
  | "onedrive"
  | "sftp"
  | "ftp"
  | "local-fs";
export type StorageBackend =
  | "s3"
  | "b2"
  | "backblaze_b2"
  | "oss"
  | "aliyun_oss"
  | "cos"
  | "tencent_cos"
  | "obs"
  | "huawei_obs"
  | "azure_blob"
  | "azblob"
  | "webdav"
  | "gcs"
  | "gdrive"
  | "google_drive"
  | "onedrive"
  | "one_drive"
  | "sftp"
  | "ftp"
  | "local"
  | "fs";
export type McpTransport = "stdio" | "http";

export interface StorageDraft {
  storageId?: string;
  name: string;
  backend: StorageBackend;
  config: Record<string, unknown>;
  enabled: boolean;
  mcpExposed: boolean;
  readOnly: boolean;
  secretMutations?: Record<string, SecretMutation>;
  oauthSessionId?: string | null;
}

export type SecretMutation =
  | { action: "keep" }
  | { action: "set"; value: string }
  | { action: "clear" };

export type AuthTokenMutation =
  | { action: "keep" }
  | { action: "set"; value: string }
  | { action: "clear" };

export interface StorageConfig extends StorageDraft {
  id: string;
  type: StorageType;
  connected: boolean;
  createdAt: string;
  updatedAt: string;
  mcpPolicy: McpStoragePolicy;
}

export type McpAccessMode = "none" | "read_only" | "read_write";

export interface McpConfirmationRules {
  require_for_write: boolean;
  require_for_overwrite: boolean;
  require_for_delete: boolean;
  require_for_version_delete: boolean;
  require_for_presign: boolean;
  require_for_cross_storage_copy: boolean;
}

export type McpRuleSource =
  | { kind: "manual" }
  | { kind: "workspace"; workspace_id: string };

export interface McpPathRule {
  id: string;
  prefix: string;
  access: McpAccessMode;
  source: McpRuleSource;
  confirmation_rules?: McpConfirmationRules;
}

export interface McpStoragePolicy {
  version: number;
  default_access: McpAccessMode;
  rules: McpPathRule[];
  denied_paths: string[];
  confirmation_rules: McpConfirmationRules;
  allowed_paths?: string[];
}

export interface StorageValidationCapabilities {
  list: boolean;
  stat: boolean;
  read: boolean;
  write: boolean;
  delete: boolean;
  copy: boolean;
  rename: boolean;
  presign_read: boolean;
  create_dir: boolean;
  write_with_user_metadata: boolean;
  list_with_versions: boolean;
  read_with_version: boolean;
  delete_with_version: boolean;
}

export interface StorageValidationResult {
  valid: boolean;
  details: string;
  capabilities: StorageValidationCapabilities;
  fix_hints: string[];
  warnings: string[];
}

export interface McpSettings {
  enabled: boolean;
  transport: McpTransport;
  bindAddress: string;
  port: number;
  enabledTools: string[];
  securityBaselineVersion: number;
  authTokenConfigured: boolean;
}

export interface McpSettingsUpdate {
  enabled: boolean;
  transport: McpTransport;
  bindAddress: string;
  port: number;
  enabledTools: string[];
  authTokenMutation: AuthTokenMutation;
}

export interface McpRuntimeStatus {
  settings: McpSettings;
  runningHttp: boolean;
  endpoint: string | null;
  endpointDisplay: string;
  authTokenConfigured: boolean;
}

export interface McpClientSnippets {
  stdio: string;
  http: string;
}

export type McpToolCategory = "read" | "write" | "destructive" | "external_link" | "session";
export type McpToolRisk = "low" | "medium" | "high";

export interface McpToolDefinition {
  name: string;
  description: string;
  category: McpToolCategory;
  risk: McpToolRisk;
  defaultEnabled: boolean;
}

export interface StorageCapabilities {
  list_with_versions: boolean;
  read_with_version: boolean;
  delete_with_version: boolean;
  presign_read: boolean;
  write_with_user_metadata?: boolean;
}

export interface FileItem {
  id: string;
  name: string;
  type: "file" | "folder";
  size?: number;
  modified: Date | null;
  owner?: string;
  extension?: string;
  capabilities?: StorageCapabilities;
}

export interface AppSettings {
  onboardingCompleted: boolean;
  onboardingSkipped: boolean;
  onboardingCompletedAt: string | null;
  onboardingSkippedAt: string | null;
  wizardStep: string | null;
  wizardCompletedSteps: string[];
  telemetryConsent: boolean | null;
}

export interface McpAuditExportManifest {
  secretsIncluded: boolean;
  fileContentsIncluded: boolean;
  authTokensIncluded: boolean;
  presignedUrlQueryStrings: string;
}

export interface McpAuditExportResult {
  path: string;
  eventCount: number;
  redactionManifest: McpAuditExportManifest;
}

export interface McpAuditEvent {
  id: string;
  timestamp: string;
  actor_type: string;
  mcp_client_id: string | null;
  session_id: string | null;
  storage_id: string | null;
  storage_name: string | null;
  backend: string | null;
  tool_name: string;
  operation: string;
  path: string | null;
  version_id: string | null;
  decision: string;
  matched_rule_id?: string | null;
  workspace_id?: string | null;
  confirmation_id: string | null;
  duration_ms: number | null;
  bytes_read: number | null;
  bytes_written: number | null;
  error_code: string | null;
}

export interface PendingMcpConfirmation {
  operation_id: string;
  tool_name: string;
  operation: string;
  risk_type: string;
  storage_id: string;
  storage_name: string;
  path: string;
  summary: string;
  created_at: string;
  expires_at: string;
}

export interface ActiveMcpSession {
  id: string;
  allowed_storages: string[];
  allowed_prefixes: string[];
  read_only: boolean;
  created_at: string;
  expires_at: string;
}

export interface ListEntriesPage {
  entries: Entry[];
  nextCursor: string | null;
  truncated: boolean;
}

export interface ReadFileRangeResult {
  totalSize: number;
  offset: number;
  bytes: number[];
  truncated: boolean;
}

export interface Entry {
  path: string;
  name: string;
  is_dir: boolean;
  size: number;
  modified_at: string | null;
  etag: string | null;
}
