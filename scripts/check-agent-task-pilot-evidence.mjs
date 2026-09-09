#!/usr/bin/env node
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";

const args = process.argv.slice(2);
const allowSynthetic = args.includes("--allow-synthetic");
const evidencePath = args.find((arg) => !arg.startsWith("--"));

const fail = (message) => {
  console.error(`Agent Task pilot evidence check failed: ${message}`);
  process.exit(1);
};

if (!evidencePath) {
  fail("usage: node scripts/check-agent-task-pilot-evidence.mjs <evidence.json> [--allow-synthetic]");
}

let evidence;
try {
  evidence = JSON.parse(fs.readFileSync(evidencePath, "utf8"));
} catch (error) {
  fail(`could not parse ${evidencePath}: ${error.message}`);
}

const isObject = (value) => value !== null && typeof value === "object" && !Array.isArray(value);
const sha256 = /^[0-9a-f]{64}$/;
const commitSha = /^[0-9a-f]{40}$/;
const stableSemver = /^\d+\.\d+\.\d+$/;
const prereleaseSemver = /^\d+\.\d+\.\d+-[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*$/;
const uuid = /^[0-9a-f]{8}-[0-9a-f]{4}-[1-8][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/;

const assertObject = (value, label) => {
  if (!isObject(value)) fail(`${label} must be an object`);
};

const assertExactKeys = (value, allowed, label) => {
  assertObject(value, label);
  const allowedSet = new Set(allowed);
  for (const key of Object.keys(value)) {
    if (!allowedSet.has(key)) fail(`${label} contains unsupported field ${key}`);
  }
  for (const key of allowed) {
    if (!(key in value)) fail(`${label} is missing required field ${key}`);
  }
};

const assertString = (value, label) => {
  if (typeof value !== "string" || value.trim() === "") fail(`${label} must be a non-empty string`);
};

const assertPositiveInteger = (value, label) => {
  if (!Number.isSafeInteger(value) || value <= 0) fail(`${label} must be a positive integer`);
};

const forbiddenKeys = new Set([
  "sourcePath",
  "hostPath",
  "sourceStorageId",
  "storageConfig",
  "credential",
  "credentials",
  "token",
  "accessToken",
  "refreshToken",
  "clientSecret",
  "secret",
  "content",
  "contents",
]);

const scanForbiddenKeys = (value, location = "evidence") => {
  if (Array.isArray(value)) {
    value.forEach((entry, index) => scanForbiddenKeys(entry, `${location}[${index}]`));
    return;
  }
  if (!isObject(value)) return;
  for (const [key, entry] of Object.entries(value)) {
    if (forbiddenKeys.has(key)) fail(`${location}.${key} is forbidden in pilot evidence`);
    scanForbiddenKeys(entry, `${location}.${key}`);
  }
};

scanForbiddenKeys(evidence);

assertExactKeys(
  evidence,
  [
    "schemaVersion",
    "synthetic",
    "candidate",
    "client",
    "tasks",
    "safetyProbes",
    "upgrade",
    "overallPassed",
    "observations",
  ],
  "evidence",
);
if (evidence.schemaVersion !== 1) fail("schemaVersion must be 1");
if (typeof evidence.synthetic !== "boolean") fail("synthetic must be a boolean");
if (evidence.synthetic && !allowSynthetic) {
  fail("synthetic evidence is not accepted as real pilot evidence; pass --allow-synthetic only for fixtures");
}

assertExactKeys(evidence.candidate, ["version", "commit", "platform", "installedFrom"], "candidate");
if (!prereleaseSemver.test(evidence.candidate.version)) fail("candidate.version must be a SemVer prerelease");
if (!commitSha.test(evidence.candidate.commit)) fail("candidate.commit must be a lowercase 40-character Git commit SHA");
if (!new Set(["linux", "macos", "windows"]).has(evidence.candidate.platform)) {
  fail("candidate.platform must be linux, macos, or windows");
}
if (!stableSemver.test(evidence.candidate.installedFrom)) fail("candidate.installedFrom must be a stable SemVer version");

if (!evidence.synthetic) {
  const tag = `v${evidence.candidate.version}`;
  let taggedCommit;
  try {
    taggedCommit = execFileSync("git", ["rev-parse", `${tag}^{commit}`], {
      encoding: "utf8",
      stdio: ["ignore", "pipe", "pipe"],
    }).trim();
  } catch {
    fail(`real pilot validation requires candidate tag ${tag} in the current Git checkout`);
  }
  if (taggedCommit !== evidence.candidate.commit) {
    fail(`candidate.commit does not match ${tag}: expected ${taggedCommit}`);
  }
}

assertExactKeys(evidence.client, ["name", "handoffViaInfimountMcp"], "client");
if (evidence.client.name !== "codex") fail("client.name must be codex for the v1 pilot protocol");
if (evidence.client.handoffViaInfimountMcp !== true) fail("client.handoffViaInfimountMcp must be true");

if (!Array.isArray(evidence.tasks) || evidence.tasks.length < 3) fail("tasks must contain at least three pilot runs");
const requiredClasses = new Set(["coding", "document", "data-analysis"]);
const seenClasses = new Set();

for (const [index, task] of evidence.tasks.entries()) {
  const label = `tasks[${index}]`;
  assertExactKeys(
    task,
    [
      "class",
      "taskId",
      "sourceStorageKind",
      "workspaceStorageKind",
      "sourceMcpExposedBefore",
      "sourceMcpExposedAfter",
      "sourceSelectionDigestBefore",
      "sourceSelectionDigestAfter",
      "preflight",
      "review",
      "publication",
      "uiEvidence",
      "passed",
    ],
    label,
  );

  if (!requiredClasses.has(task.class)) fail(`${label}.class must be coding, document, or data-analysis`);
  if (seenClasses.has(task.class)) fail(`${label}.class duplicates pilot class ${task.class}`);
  seenClasses.add(task.class);
  if (!uuid.test(task.taskId)) fail(`${label}.taskId must be a lowercase UUID`);
  assertString(task.sourceStorageKind, `${label}.sourceStorageKind`);
  if (task.workspaceStorageKind !== "local") fail(`${label}.workspaceStorageKind must be local for v0.9.0`);
  if (typeof task.sourceMcpExposedBefore !== "boolean") fail(`${label}.sourceMcpExposedBefore must be a boolean`);
  if (typeof task.sourceMcpExposedAfter !== "boolean") fail(`${label}.sourceMcpExposedAfter must be a boolean`);
  if (task.sourceMcpExposedBefore !== task.sourceMcpExposedAfter) {
    fail(`${label} changed source MCP exposure during task preparation`);
  }
  if (!sha256.test(task.sourceSelectionDigestBefore)) fail(`${label}.sourceSelectionDigestBefore must be lowercase SHA-256`);
  if (!sha256.test(task.sourceSelectionDigestAfter)) fail(`${label}.sourceSelectionDigestAfter must be lowercase SHA-256`);
  if (task.sourceSelectionDigestBefore !== task.sourceSelectionDigestAfter) {
    fail(`${label} source selection digest changed during the pilot`);
  }

  assertExactKeys(task.preflight, ["selectedItems", "fileCount", "totalBytes"], `${label}.preflight`);
  assertPositiveInteger(task.preflight.selectedItems, `${label}.preflight.selectedItems`);
  assertPositiveInteger(task.preflight.fileCount, `${label}.preflight.fileCount`);
  assertPositiveInteger(task.preflight.totalBytes, `${label}.preflight.totalBytes`);

  assertExactKeys(task.review, ["fileCount", "totalBytes", "nothingSelectedByDefault"], `${label}.review`);
  assertPositiveInteger(task.review.fileCount, `${label}.review.fileCount`);
  assertPositiveInteger(task.review.totalBytes, `${label}.review.totalBytes`);
  if (task.review.nothingSelectedByDefault !== true) fail(`${label}.review.nothingSelectedByDefault must be true`);

  assertExactKeys(
    task.publication,
    [
      "selectedCount",
      "conflictPolicy",
      "overwriteAvailable",
      "previewApproved",
      "destinationVerified",
      "receiptPath",
      "receiptSha256",
    ],
    `${label}.publication`,
  );
  assertPositiveInteger(task.publication.selectedCount, `${label}.publication.selectedCount`);
  if (task.publication.selectedCount > task.review.fileCount) {
    fail(`${label}.publication.selectedCount cannot exceed reviewed output count`);
  }
  if (!new Set(["fail", "rename"]).has(task.publication.conflictPolicy)) {
    fail(`${label}.publication.conflictPolicy must be fail or rename`);
  }
  if (task.publication.overwriteAvailable !== false) fail(`${label}.publication.overwriteAvailable must be false`);
  if (task.publication.previewApproved !== true) fail(`${label}.publication.previewApproved must be true`);
  if (task.publication.destinationVerified !== true) fail(`${label}.publication.destinationVerified must be true`);
  const receiptPrefix = `tasks/${task.taskId}/publish-receipt-`;
  if (
    typeof task.publication.receiptPath !== "string" ||
    !task.publication.receiptPath.startsWith(receiptPrefix) ||
    !task.publication.receiptPath.endsWith(".json")
  ) {
    fail(`${label}.publication.receiptPath must be a unique receipt under the task root`);
  }
  if (!sha256.test(task.publication.receiptSha256)) fail(`${label}.publication.receiptSha256 must be lowercase SHA-256`);

  if (!Array.isArray(task.uiEvidence) || task.uiEvidence.length < 2) {
    fail(`${label}.uiEvidence must contain at least review and publication evidence references`);
  }
  for (const [evidenceIndex, ref] of task.uiEvidence.entries()) {
    assertString(ref, `${label}.uiEvidence[${evidenceIndex}]`);
    if (path.isAbsolute(ref) || ref.split(/[\\/]/).includes("..")) {
      fail(`${label}.uiEvidence[${evidenceIndex}] must be a safe relative reference`);
    }
  }
  if (task.passed !== true) fail(`${label}.passed must be true for an accepted pilot bundle`);
}

for (const requiredClass of requiredClasses) {
  if (!seenClasses.has(requiredClass)) fail(`tasks is missing required ${requiredClass} pilot`);
}

assertExactKeys(
  evidence.safetyProbes,
  ["failConflictRejected", "stalePreviewRejected", "overwriteUnavailable"],
  "safetyProbes",
);
for (const field of ["failConflictRejected", "stalePreviewRejected", "overwriteUnavailable"]) {
  if (evidence.safetyProbes[field] !== true) fail(`safetyProbes.${field} must be true`);
}

assertExactKeys(
  evidence.upgrade,
  ["from", "to", "configurationRetained", "storageRegistryRetained", "workspaceRegistryRetained", "agentTasksVisible", "passed"],
  "upgrade",
);
if (evidence.upgrade.from !== evidence.candidate.installedFrom) fail("upgrade.from must match candidate.installedFrom");
if (evidence.upgrade.to !== evidence.candidate.version) fail("upgrade.to must match candidate.version");
for (const field of ["configurationRetained", "storageRegistryRetained", "workspaceRegistryRetained", "agentTasksVisible", "passed"]) {
  if (evidence.upgrade[field] !== true) fail(`upgrade.${field} must be true`);
}

if (evidence.overallPassed !== true) fail("overallPassed must be true for an accepted pilot bundle");
if (!Array.isArray(evidence.observations)) fail("observations must be an array");
for (const [index, observation] of evidence.observations.entries()) {
  assertString(observation, `observations[${index}]`);
}

console.log(
  `Agent Task pilot evidence passed for ${evidence.candidate.version} on ${evidence.candidate.platform}: ${[...seenClasses].sort().join(", ")}`,
);
