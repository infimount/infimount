export type SourceKind = "local" | "s3" | "webdav" | "azure_blob" | "gcs" | "b2" | "oss" | "cos" | "obs";

export interface Source {
  id: string;
  name: string;
  kind: SourceKind;
  root: string;
  config: any;
}
