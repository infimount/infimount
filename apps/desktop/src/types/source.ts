export type SourceKind = "local" | "s3" | "webdav" | "azure_blob" | "gcs" | "b2";

export interface Source {
  id: string;
  name: string;
  kind: SourceKind;
  root: string;
  config?: Record<string, string>;
}
