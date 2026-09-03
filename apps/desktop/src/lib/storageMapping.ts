import type { StorageBackend, StorageType } from "@/types/storage";

const BACKEND_TO_TYPE: Record<StorageBackend, StorageType> = {
  local: "local-fs",
  fs: "local-fs",
  s3: "aws-s3",
  b2: "backblaze-b2",
  backblaze_b2: "backblaze-b2",
  oss: "aliyun-oss",
  aliyun_oss: "aliyun-oss",
  cos: "tencent-cos",
  tencent_cos: "tencent-cos",
  obs: "huawei-obs",
  huawei_obs: "huawei-obs",
  azure_blob: "azure-blob",
  azblob: "azure-blob",
  webdav: "webdav",
  gcs: "gcs",
  gdrive: "google-drive",
  google_drive: "google-drive",
  onedrive: "onedrive",
  one_drive: "onedrive",
  sftp: "sftp",
  ftp: "ftp",
};

export function backendToStorageType(backend: StorageBackend): StorageType {
  return BACKEND_TO_TYPE[backend] ?? "local-fs";
}
