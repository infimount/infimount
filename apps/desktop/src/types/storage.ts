export type StorageType = "aws-s3" | "azure-blob" | "webdav" | "gcs" | "local-fs";
export type StorageBackend = "s3" | "azure_blob" | "webdav" | "gcs" | "local";
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
}

export interface StorageValidationResult {
  valid: boolean;
  details: string;
  capabilities: StorageValidationCapabilities;
}

export interface McpSettings {
  enabled: boolean;
  transport: McpTransport;
  bindAddress: string;
  port: number;
  enabledTools: string[];
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

export interface FileItem {
  id: string;
  name: string;
  type: "file" | "folder";
  size?: number;
  modified: Date | null;
  owner?: string;
  extension?: string;
}
