#!/usr/bin/env node
import fs from "node:fs";

const fail = (message) => {
  console.error(`feature docs check failed: ${message}`);
  process.exit(1);
};

const read = (path) => fs.readFileSync(path, "utf8");
const schemas = JSON.parse(read("crates/core/storage_schemas.json"));
const docFiles = [
  "README.md",
  "docs/index.html",
  "docs/backend-capabilities.md",
  "docs/llms.txt",
  "Agents.md",
];

const expectedByKind = new Map([
  ["local", ["Local"]],
  ["s3", ["S3-compatible"]],
  ["b2", ["Backblaze B2"]],
  ["oss", ["Aliyun OSS"]],
  ["cos", ["Tencent COS"]],
  ["obs", ["Huawei OBS"]],
  ["azure_blob", ["Azure Blob"]],
  ["gcs", ["Google Cloud Storage"]],
  ["webdav", ["WebDAV"]],
]);

for (const schema of schemas) {
  const expected = expectedByKind.get(schema.kind) ?? [schema.label];
  for (const file of docFiles) {
    const text = read(file);
    if (!expected.some((needle) => text.includes(needle))) {
      fail(`${file} should mention ${schema.kind} using one of: ${expected.join(", ")}`);
    }
  }
}

const webpage = read("docs/index.html");
if (webpage.includes("<span>download_link</span>")) {
  fail("docs/index.html must use the real MCP tool name generate_download_link, not download_link");
}
if (!webpage.includes("S3/S3-compatible")) {
  fail("docs/index.html should describe S3 as S3/S3-compatible in visible copy");
}

const readme = read("README.md");
for (const section of ["## Workbench", "## Agent Workspaces", "## Agent Tasks"]) {
  if (!readme.includes(section)) fail(`README.md is missing ${section}`);
}

for (const phrase of [
  "dual-pane",
  "transfer queue",
  "workspace-scoped MCP policy",
  "plain workspace",
  "storage-relative",
  "memory files",
  "checkpoints",
  "review",
  "publish",
  "create-only",
  "product-validation phase",
]) {
  if (!readme.toLowerCase().includes(phrase.toLowerCase())) {
    fail(`README.md should mention ${phrase}`);
  }
}
if (readme.includes("Create coding, research, or data-analysis workspaces")) {
  fail("README.md must not describe agent-type templates as the new Agent Workspace creation model");
}
if (/pilot evidence remains the v\d+\.\d+\.\d+ release gate/i.test(readme)) {
  fail("README.md must not make real pilot evidence a manual release-test gate");
}

if (!fs.existsSync("docs/agent-workspaces.md")) {
  fail("docs/agent-workspaces.md is missing");
}
const agentWorkspaces = read("docs/agent-workspaces.md");
for (const phrase of [
  "storage-scoped MCP boundary",
  "New workspaces are plain scoped folders",
  "does not ask for an agent type or starter template",
  "explicit absolute host path",
]) {
  if (!agentWorkspaces.toLowerCase().includes(phrase.toLowerCase())) {
    fail(`docs/agent-workspaces.md should mention ${phrase}`);
  }
}

if (!fs.existsSync("docs/agent-tasks.md")) {
  fail("docs/agent-tasks.md is missing");
}
const agentTasks = read("docs/agent-tasks.md");
if (!agentTasks.includes("targeted for **v0.8.1**")) {
  fail("docs/agent-tasks.md must preserve the planned v0.8.1 Agent Tasks release target");
}
if (agentTasks.includes("publish-receipt.json")) {
  fail("docs/agent-tasks.md must document unique publication receipts, not a mutable static receipt");
}
if (/remaining v\d+\.\d+\.\d+ gate is \*\*pilot evidence\*\*/i.test(agentTasks)) {
  fail("docs/agent-tasks.md must keep pilot evidence separate from the automated release gate");
}
for (const phrase of [
  "v0.8.1",
  "publish-receipt-<publication-id>.json",
  "create-only",
  "fail",
  "rename",
  "no overwrite mode",
  "cleanup-required",
  "product-validation phase",
  "not a manual product-test requirement in the automated release gate",
  "agent-tasks-pilot.md",
]) {
  if (!agentTasks.toLowerCase().includes(phrase.toLowerCase())) {
    fail(`docs/agent-tasks.md should mention ${phrase}`);
  }
}

if (!fs.existsSync("docs/agent-tasks-pilot.md")) {
  fail("docs/agent-tasks-pilot.md is missing");
}
const pilot = read("docs/agent-tasks-pilot.md");
for (const phrase of [
  "product-validation artifact",
  "synthetic fixture",
  "v0.8.1-rc.2",
  "stale approved preview",
  "fail-on-conflict rejection",
  "check-agent-task-pilot-evidence.mjs",
  "must never be presented as a completed pilot",
]) {
  if (!pilot.toLowerCase().includes(phrase.toLowerCase())) {
    fail(`docs/agent-tasks-pilot.md should mention ${phrase}`);
  }
}

const llms = read("docs/llms.txt");
if (!llms.includes("docs/agent-tasks.md")) {
  fail("docs/llms.txt should link to the Agent Tasks contract");
}

console.log("Feature docs check passed.");
