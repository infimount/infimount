export type StorageType = 'aws-s3' | 'azure-blob' | 'webdav' | 'local-fs';

export interface StorageConfig {
  id: string;
  name: string;
  type: StorageType;
  config: Record<string, string>;
  connected: boolean;
}

export interface FileItem {
  id: string;
  name: string;
  type: 'file' | 'folder';
  size?: number;
  modified: Date | null;
  owner?: string;
  extension?: string;
}
