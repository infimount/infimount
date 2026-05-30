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
for (const section of ["## Workbench", "## Agent Workspaces"]) {
  if (!readme.includes(section)) fail(`README.md is missing ${section}`);
}

for (const phrase of ["dual-pane", "transfer queue", "workspace-scoped MCP policy", "memory files", "checkpoints"]) {
  if (!readme.toLowerCase().includes(phrase.toLowerCase())) {
    fail(`README.md should mention ${phrase}`);
  }
}

console.log("Feature docs check passed.");
