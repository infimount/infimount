import { describe, expect, it } from "vitest";

import { backendToStorageType } from "./storageMapping";

describe("backendToStorageType", () => {
  it.each([
    ["oss", "aliyun-oss"],
    ["aliyun_oss", "aliyun-oss"],
    ["cos", "tencent-cos"],
    ["tencent_cos", "tencent-cos"],
    ["obs", "huawei-obs"],
    ["huawei_obs", "huawei-obs"],
    ["gdrive", "google-drive"],
    ["google_drive", "google-drive"],
    ["onedrive", "onedrive"],
    ["one_drive", "onedrive"],
    ["sftp", "sftp"],
  ])("maps %s backend to %s storage type", (backend, type) => {
    expect(backendToStorageType(backend as never)).toBe(type);
  });
});
