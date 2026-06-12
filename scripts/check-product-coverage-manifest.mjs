#!/usr/bin/env node
import { existsSync, readFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const rootDir = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const manifestPath = path.join(rootDir, "docs", "product-coverage-manifest.json");

function fail(message) {
  console.error(`product coverage manifest check failed: ${message}`);
  process.exit(1);
}

if (!existsSync(manifestPath)) {
  fail("docs/product-coverage-manifest.json is missing");
}

let manifest;
try {
  manifest = JSON.parse(readFileSync(manifestPath, "utf8"));
} catch (error) {
  fail(`manifest is not valid JSON: ${error instanceof Error ? error.message : String(error)}`);
}

if (!Array.isArray(manifest.requiredFlows) || manifest.requiredFlows.length === 0) {
  fail("manifest.requiredFlows must be a non-empty array");
}

const seenIds = new Set();
for (const [index, flow] of manifest.requiredFlows.entries()) {
  if (!flow || typeof flow !== "object") {
    fail(`requiredFlows[${index}] must be an object`);
  }
  if (typeof flow.id !== "string" || !flow.id.trim()) {
    fail(`requiredFlows[${index}].id must be a non-empty string`);
  }
  if (seenIds.has(flow.id)) {
    fail(`duplicate flow id: ${flow.id}`);
  }
  seenIds.add(flow.id);

  if (typeof flow.path !== "string" || !flow.path.trim()) {
    fail(`requiredFlows[${index}].path must be a non-empty string`);
  }
  if (path.isAbsolute(flow.path) || flow.path.includes("..")) {
    fail(`${flow.id} path must be repo-relative and must not contain '..'`);
  }
  const targetPath = path.join(rootDir, flow.path);
  if (!existsSync(targetPath)) {
    fail(`${flow.id} points to missing file: ${flow.path}`);
  }
  if (typeof flow.reason !== "string" || !flow.reason.trim()) {
    fail(`${flow.id} must explain why the behavior is required`);
  }
}

console.log(`Product coverage manifest check passed (${manifest.requiredFlows.length} required flows).`);
