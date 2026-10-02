#!/usr/bin/env node

import fs from "node:fs";
import path from "node:path";
import { execFileSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const ROOT_DIR = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");

function fail(message) {
  console.error("RC10 pilot acceptance failed: " + message);
  process.exit(1);
}

function arg(name) {
  const index = process.argv.indexOf(name);
  return index >= 0 ? process.argv[index + 1] : undefined;
}

if (!process.argv.includes("--accept-quality")) {
  fail("--accept-quality is required; automated checks cannot make the usefulness judgment");
}

const rootArg = arg("--root");
if (!rootArg) fail("--root <pilot-root> is required");
const pilotRoot = path.resolve(rootArg);
const evidenceDir = path.join(pilotRoot, "evidence");
const pendingFile = path.join(evidenceDir, "pilot-evidence.pending.json");
const automatedFile = path.join(evidenceDir, "automated-results.json");

if (!fs.existsSync(pendingFile) || !fs.existsSync(automatedFile)) {
  fail("pending evidence or automated results are missing");
}

const automated = JSON.parse(fs.readFileSync(automatedFile, "utf8"));
if (automated.allAutomatedChecksPassed !== true || automated.qualityAcceptancePending !== true) {
  fail("automated pilot checks are not in the expected quality-pending state");
}

const evidence = JSON.parse(fs.readFileSync(pendingFile, "utf8"));
if (evidence.synthetic !== false || evidence.overallPassed !== false) {
  fail("pending evidence is not in the expected unaccepted state");
}
if (!Array.isArray(evidence.tasks) || evidence.tasks.length !== 3) {
  fail("pending evidence does not contain exactly three canonical workloads");
}
for (const task of evidence.tasks) {
  if (task.passed !== false) {
    fail("a pending task was already marked passed");
  }
  task.passed = true;
}
evidence.overallPassed = true;
evidence.observations.push(
  "Human reviewed the three deterministic workload outputs and accepted their usefulness and correctness.",
);

const finalFile = path.join(evidenceDir, "pilot-evidence.json");
fs.writeFileSync(finalFile, JSON.stringify(evidence, null, 2) + "\n", { mode: 0o600 });

try {
  execFileSync(
    process.execPath,
    [path.join(ROOT_DIR, "scripts", "check-agent-task-pilot-evidence.mjs"), finalFile],
    { cwd: ROOT_DIR, stdio: "inherit" },
  );
} catch {
  fs.rmSync(finalFile, { force: true });
  fail("final evidence validator rejected the accepted bundle");
}

automated.qualityAcceptancePending = false;
automated.qualityAccepted = true;
fs.writeFileSync(automatedFile, JSON.stringify(automated, null, 2) + "\n", { mode: 0o600 });

console.log("========================================");
console.log("RC10 REAL PILOT EVIDENCE ACCEPTED");
console.log("candidate=0.8.1-rc.10");
console.log("workloads=coding,document,data-analysis");
console.log("automated_checks=passed");
console.log("human_quality_acceptance=yes");
console.log("evidence=" + finalFile);
console.log("========================================");
