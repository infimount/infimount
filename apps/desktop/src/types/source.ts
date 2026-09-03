export type SourceKind = "local" | "s3" | "webdav" | "azure_blob" | "gcs" | "b2" | "oss" | "cos" | "obs" | "sftp" | "gdrive" | "onedrive";

export interface Source {
  id: string;
  name: string;
  kind: SourceKind;
  root: string;
  config?: Record<string, string>;
}
