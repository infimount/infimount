export type SourceKind =
  | "local"
  | "s3"
  | "webdav"
  | "azure_blob"
  | "gcs";

export interface Source {
  id: string;
  name: string;
  kind: SourceKind;
  root: string;
}

