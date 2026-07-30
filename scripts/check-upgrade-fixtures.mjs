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
