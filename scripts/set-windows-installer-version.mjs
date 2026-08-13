#!/usr/bin/env node
import fs from "node:fs";
import { canonicalVersionFromTag, windowsMsiVersion } from "./release-version-utils.mjs";

const tag = process.env.GITHUB_REF_NAME ?? "";
const canonicalVersion = canonicalVersionFromTag(tag);
const msiVersion = windowsMsiVersion(canonicalVersion);
const configPath = "apps/desktop/src-tauri/tauri.conf.json";
const config = JSON.parse(fs.readFileSync(configPath, "utf8"));

if (config.version !== canonicalVersion) {
  throw new Error(
    `canonical app version ${config.version} does not match release tag ${canonicalVersion}; run sync-release-version first`,
  );
}
config.bundle ??= {};
config.bundle.windows ??= {};
config.bundle.windows.wix ??= {};
config.bundle.windows.wix.version = msiVersion;
fs.writeFileSync(configPath, `${JSON.stringify(config, null, 2)}\n`, "utf8");

console.log(`Windows MSI version set to ${msiVersion}; canonical app version remains ${canonicalVersion}`);
