#!/usr/bin/env node
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { canonicalVersionFromTag, windowsMsiVersion } from "./release-version-utils.mjs";

assert.equal(canonicalVersionFromTag("v0.8.0-rc.1"), "0.8.0-rc.1");
assert.equal(windowsMsiVersion("0.8.0-rc.1"), "0.8.0.1");
assert.equal(windowsMsiVersion("0.8.0"), "0.8.0");
assert.throws(() => windowsMsiVersion("0.8.0-rc"), /numeric identifier/);
assert.throws(() => canonicalVersionFromTag("v0.8.0+local"), /unsupported release tag/);

const root = path.resolve(import.meta.dirname, "..");
const temp = fs.mkdtempSync(path.join(os.tmpdir(), "infimount-release-version-"));
try {
  fs.mkdirSync(path.join(temp, "apps/desktop/src-tauri"), { recursive: true });
  fs.writeFileSync(path.join(temp, "package.json"), '{"version":"0.0.0"}\n');
  fs.writeFileSync(path.join(temp, "apps/desktop/package.json"), '{"version":"0.0.0"}\n');
  fs.writeFileSync(
    path.join(temp, "apps/desktop/src-tauri/tauri.conf.json"),
    '{"version":"0.0.0","bundle":{}}\n',
  );
  fs.writeFileSync(path.join(temp, "Cargo.toml"), '[workspace.package]\nversion = "0.0.0"\n');

  const env = { ...process.env, GITHUB_REF_NAME: "v0.8.0-rc.1" };
  execFileSync("node", [path.join(root, "scripts/sync-release-version.mjs")], { cwd: temp, env });
  execFileSync("node", [path.join(root, "scripts/set-windows-installer-version.mjs")], {
    cwd: temp,
    env,
  });

  assert.equal(JSON.parse(fs.readFileSync(path.join(temp, "package.json"))).version, "0.8.0-rc.1");
  const tauri = JSON.parse(
    fs.readFileSync(path.join(temp, "apps/desktop/src-tauri/tauri.conf.json")),
  );
  assert.equal(tauri.version, "0.8.0-rc.1");
  assert.equal(tauri.bundle.windows.wix.version, "0.8.0.1");
  assert.match(fs.readFileSync(path.join(temp, "Cargo.toml"), "utf8"), /0\.8\.0-rc\.1/);
} finally {
  fs.rmSync(temp, { recursive: true, force: true });
}

console.log("Release version model check passed.");
