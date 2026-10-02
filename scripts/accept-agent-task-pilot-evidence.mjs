#!/usr/bin/env node
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const ROOT_DIR = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");

const fail = (message) => {
  console.error(`Agent Task pilot acceptance failed: ${message}`);
  process.exit(1);
};

const arg = (name) => {
  const index = process.argv.indexOf(name);
  return index >= 0 ? process.argv[index + 1] : undefined;
};

if (!process.argv.includes("--accept-quality")) {
  fail("--accept-quality is required; automated checks cannot make the usefulness judgment");
}

const evidenceArg = arg("--evidence");
if (!evidenceArg) fail("--evidence <pilot-evidence.pending.json> is required");
const pendingFile = path.resolve(evidenceArg);
if (!fs.existsSync(pendingFile)) fail(`pending evidence is missing: ${pendingFile}`);

const pending = JSON.parse(fs.readFileSync(pendingFile, "utf8"));
if (pending.synthetic !== false) fail("real quality acceptance cannot be applied to synthetic evidence");
if (!Array.isArray(pending.tasks) || pending.tasks.length !== 3) {
  fail("pending evidence must contain exactly three canonical workloads");
}
if (pending.tasks.some((task) => task.passed !== false) || pending.overallPassed !== false) {
  fail("pending evidence is not in the expected quality-pending state");
}

execFileSync(
  process.execPath,
  [
    path.join(ROOT_DIR, "scripts", "check-agent-task-pilot-evidence.mjs"),
    pendingFile,
    "--allow-pending-quality",
  ],
  { cwd: ROOT_DIR, stdio: "inherit" },
);

const accepted = structuredClone(pending);
for (const task of accepted.tasks) task.passed = true;
accepted.overallPassed = true;
accepted.observations = Array.isArray(accepted.observations) ? accepted.observations : [];
accepted.observations.push(
  "Human quality acceptance explicitly recorded after reviewing all three real workload outputs and private UI evidence.",
);

const defaultOut = pendingFile.endsWith(".pending.json")
  ? pendingFile.slice(0, -".pending.json".length) + ".json"
  : pendingFile + ".accepted.json";
const outFile = path.resolve(arg("--out") || defaultOut);
if (outFile === pendingFile) fail("accepted evidence output must not overwrite the pending evidence file");

fs.writeFileSync(outFile, JSON.stringify(accepted, null, 2) + "\n", { mode: 0o600 });

try {
  execFileSync(
    process.execPath,
    [path.join(ROOT_DIR, "scripts", "check-agent-task-pilot-evidence.mjs"), outFile],
    { cwd: ROOT_DIR, stdio: "inherit" },
  );
} catch (error) {
  fs.rmSync(outFile, { force: true });
  throw error;
}

console.log(`Accepted real pilot evidence written to ${outFile}`);
