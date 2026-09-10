#!/usr/bin/env node

import { execFileSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

const DEFAULT_CANDIDATE_VERSION = "0.8.1-rc.4";
const INSTALLED_FROM = "0.8.0";
const SCRIPT_DIR = path.dirname(fileURLToPath(import.meta.url));
const CORE = path.join(SCRIPT_DIR, "agent-task-pilot-kit-core.mjs");

const fail = (message) => {
  console.error(`Agent Task pilot kit failed: ${message}`);
  process.exit(1);
};

const getArg = (args, name) => {
  const index = args.indexOf(name);
  return index >= 0 ? args[index + 1] : undefined;
};

const normalizeVersion = (value) => (value.startsWith("v") ? value.slice(1) : value);

const validateVersion = (version) => {
  if (!/^\d+\.\d+\.\d+(?:-[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?$/.test(version)) {
    fail(`invalid candidate version: ${version}`);
  }
};

const resolveCandidateCommit = (version) => {
  const tag = `v${version}`;
  try {
    return execFileSync("git", ["rev-parse", `${tag}^{commit}`], {
      encoding: "utf8",
      stdio: ["ignore", "pipe", "ignore"],
    }).trim();
  } catch {
    return "";
  }
};

const validateCommit = (commit, version) => {
  if (!/^[0-9a-f]{40}$/.test(commit)) {
    fail(
      `candidate commit unavailable for ${version}; fetch tag v${version} or pass --candidate-commit <40-hex-sha>`,
    );
  }
};

const runCore = (args, options = {}) =>
  execFileSync(process.execPath, [CORE, ...args], {
    encoding: "utf8",
    stdio: options.capture ? ["ignore", "pipe", "pipe"] : "inherit",
  });

const rewriteGeneratedIdentity = (out, version, commit) => {
  const evidencePath = path.join(out, "pilot-evidence.template.json");
  const runbookPath = path.join(out, "RUNBOOK.md");

  const evidence = JSON.parse(fs.readFileSync(evidencePath, "utf8"));
  if (evidence.synthetic !== false || evidence.overallPassed !== false) {
    fail("generated evidence template must start synthetic=false and overallPassed=false");
  }
  if (!Array.isArray(evidence.tasks) || evidence.tasks.some((task) => task.passed !== false)) {
    fail("generated evidence template must start with every task unpassed");
  }

  const oldVersion = evidence.candidate?.version;
  const oldCommit = evidence.candidate?.commit;
  if (!oldVersion || !oldCommit) fail("generated evidence template is missing candidate identity");

  evidence.candidate.version = version;
  evidence.candidate.commit = commit;
  evidence.candidate.installedFrom = INSTALLED_FROM;
  evidence.upgrade.from = INSTALLED_FROM;
  evidence.upgrade.to = version;
  fs.writeFileSync(evidencePath, `${JSON.stringify(evidence, null, 2)}\n`, "utf8");

  let runbook = fs.readFileSync(runbookPath, "utf8");
  runbook = runbook.split(oldVersion).join(version).split(oldCommit).join(commit);
  fs.writeFileSync(runbookPath, runbook, "utf8");

  const finalEvidence = fs.readFileSync(evidencePath, "utf8");
  const finalRunbook = fs.readFileSync(runbookPath, "utf8");
  const staleVersionRemains = oldVersion !== version && (finalEvidence.includes(oldVersion) || finalRunbook.includes(oldVersion));
  const staleCommitRemains = oldCommit !== commit && (finalEvidence.includes(oldCommit) || finalRunbook.includes(oldCommit));
  if (staleVersionRemains || staleCommitRemains) {
    fail("historical candidate identity remains in generated pilot artifacts");
  }
};

const prepare = (args) => {
  const version = normalizeVersion(getArg(args, "--candidate-version") || DEFAULT_CANDIDATE_VERSION);
  validateVersion(version);

  const explicitCommit = getArg(args, "--candidate-commit") || "";
  const taggedCommit = resolveCandidateCommit(version);
  if (explicitCommit && taggedCommit && explicitCommit !== taggedCommit) {
    fail(`candidate commit ${explicitCommit} does not match tag v${version} -> ${taggedCommit}`);
  }
  const commit = explicitCommit || taggedCommit;
  validateCommit(commit, version);

  const out = path.resolve(
    getArg(args, "--out") || path.join(os.tmpdir(), `infimount-agent-task-pilot-${version}`),
  );

  let coreSummary;
  try {
    coreSummary = JSON.parse(runCore(["prepare", "--out", out], { capture: true }));
  } catch (error) {
    const detail = error.stderr?.toString().trim() || error.message;
    fail(`core workload preparation failed: ${detail}`);
  }

  rewriteGeneratedIdentity(out, version, commit);
  coreSummary.candidate = version;
  coreSummary.commit = commit;
  console.log(JSON.stringify(coreSummary, null, 2));
};

const selfTest = () => {
  try {
    runCore(["self-test"], { capture: true });
  } catch (error) {
    const detail = error.stderr?.toString().trim() || error.message;
    fail(`core workload self-test failed: ${detail}`);
  }

  const root = path.join(os.tmpdir(), `infimount-agent-task-pilot-wrapper-self-test-${process.pid}`);
  const taggedCommit = resolveCandidateCommit(DEFAULT_CANDIDATE_VERSION);
  const commit = taggedCommit || "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
  try {
    fs.rmSync(root, { recursive: true, force: true });
    prepare([
      "--candidate-version",
      DEFAULT_CANDIDATE_VERSION,
      "--candidate-commit",
      commit,
      "--out",
      root,
    ]);
    const evidence = JSON.parse(fs.readFileSync(path.join(root, "pilot-evidence.template.json"), "utf8"));
    if (evidence.candidate.version !== DEFAULT_CANDIDATE_VERSION) {
      fail(`wrapper self-test candidate version mismatch: ${evidence.candidate.version}`);
    }
    if (evidence.candidate.commit !== commit || evidence.upgrade.to !== DEFAULT_CANDIDATE_VERSION) {
      fail("wrapper self-test candidate commit/upgrade identity mismatch");
    }
    if (evidence.overallPassed !== false || evidence.tasks.some((task) => task.passed !== false)) {
      fail("wrapper self-test evidence must remain unobserved/unpassed");
    }
    console.log("Agent Task pilot wrapper self-test passed.");
  } finally {
    fs.rmSync(root, { recursive: true, force: true });
  }
};

const args = process.argv.slice(2);
const command = args[0];

if (command === "prepare") {
  prepare(args.slice(1));
} else if (command === "digest" || command === "check") {
  runCore(args);
} else if (command === "self-test") {
  selfTest();
} else {
  fail(
    "usage: agent-task-pilot-kit.mjs prepare [--candidate-version <version>] [--candidate-commit <sha>] [--out <dir>] | digest <source-directory> | check <coding|document|data-analysis> --kit <kit-dir> --outputs <outputs-dir> | self-test",
  );
}
