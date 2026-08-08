#!/usr/bin/env node
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const fixtures = [
  "tests/fixtures/v0.7/storages-plaintext.json",
  "tests/fixtures/v0.7/mcp-settings-all-tools.json",
  "tests/fixtures/v0.7/app-settings.json",
  "tests/fixtures/v0.7/workspaces-localstorage.json",
  "tests/fixtures/v0.7.1/workspaces-localstorage.json",
  "tests/fixtures/v0.7.1/storage-policy-legacy.json",
  "tests/fixtures/v0.7.1/workspace-manifest.json",
  "tests/fixtures/v0.8/shareable-config.json",
  "tests/fixtures/v0.8/recovery-payload.json",
];
const markers = [
  "TEST_SECRET_ACCESS_KEY_DO_NOT_SHIP",
  "TEST_OAUTH_REFRESH_TOKEN_DO_NOT_SHIP",
  "TEST_HTTP_BEARER_TOKEN_DO_NOT_SHIP",
];

for (const relative of fixtures) {
  const filename = path.join(root, relative);
  if (!fs.statSync(filename).isFile()) throw new Error(`missing upgrade fixture: ${relative}`);
  JSON.parse(fs.readFileSync(filename, "utf8"));
}
const fixtureCorpus = fixtures.map((relative) => fs.readFileSync(path.join(root, relative), "utf8")).join("\n");
for (const marker of markers) {
  if (!fixtureCorpus.includes(marker)) throw new Error(`fixture corpus is missing marker: ${marker}`);
}

const legacyWorkspaces = JSON.parse(fs.readFileSync(path.join(root, "tests/fixtures/v0.7.1/workspaces-localstorage.json"), "utf8"));
const legacyPolicy = JSON.parse(fs.readFileSync(path.join(root, "tests/fixtures/v0.7.1/storage-policy-legacy.json"), "utf8"));
const legacyManifest = JSON.parse(fs.readFileSync(path.join(root, "tests/fixtures/v0.7.1/workspace-manifest.json"), "utf8"));
const legacyWorkspace = legacyWorkspaces[0];
if (legacyPolicy.version !== 1 || !legacyPolicy.allowed_paths.includes(legacyWorkspace.rootPath.replace(/^\//, ""))) {
  throw new Error("v0.7.1 fixture must contain a legacy allowed_paths policy for the workspace");
}
const adoptedRule = legacyPolicy.rules.find(
  (rule) => rule.source?.kind === "manual" && rule.prefix === legacyWorkspace.rootPath.replace(/^\//, ""),
);
if (!adoptedRule) throw new Error("v0.7.1 fixture must contain an adoptable migrated manual rule");
if (legacyManifest.workspace.id !== legacyWorkspace.id || legacyManifest.workspace.rootPath !== legacyWorkspace.rootPath) {
  throw new Error("v0.7.1 workspace manifest does not match localStorage identity");
}

function scan(target) {
  const stat = fs.statSync(target);
  if (stat.isDirectory()) {
    for (const entry of fs.readdirSync(target)) scan(path.join(target, entry));
    return;
  }
  if (!stat.isFile()) return;
  const bytes = fs.readFileSync(target);
  for (const marker of markers) {
    if (bytes.includes(Buffer.from(marker))) {
      throw new Error(`seeded fixture marker leaked into artifact: ${path.relative(root, target)}`);
    }
  }
}
for (const argument of process.argv.slice(2)) scan(path.resolve(root, argument));
console.log("Upgrade fixture and artifact-marker check passed.");
