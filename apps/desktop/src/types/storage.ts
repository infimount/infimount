export type StorageType = "aws-s3" | "backblaze-b2" | "azure-blob" | "webdav" | "gcs" | "local-fs";
export type StorageBackend = "s3" | "b2" | "azure_blob" | "webdav" | "gcs" | "local";
export type McpTransport = "stdio" | "http";

export interface StorageDraft {
  name: string;
  backend: StorageBackend;
  config: Record<string, unknown>;
  enabled: boolean;
  mcpExposed: boolean;
  readOnly: boolean;
}

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

export interface McpStoragePolicy {
  default_access: McpAccessMode;
  allowed_paths: string[];
  denied_paths: string[];
  confirmation_rules: McpConfirmationRules;
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
  authToken?: string | null;
}

export interface McpRuntimeStatus {
  settings: McpSettings;
  runningHttp: boolean;
  endpoint: string | null;
  endpointDisplay: string;
}

export interface McpClientSnippets {
  stdio: string;
  http: string;
}

export interface McpToolDefinition {
  name: string;
  description: string;
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
