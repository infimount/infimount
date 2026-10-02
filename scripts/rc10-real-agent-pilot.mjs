#!/usr/bin/env node

import crypto from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import { execFileSync, spawn, spawnSync } from "node:child_process";
import { setTimeout as sleep } from "node:timers/promises";
import { fileURLToPath } from "node:url";

import {
  ELEMENT_KEY,
  TauriWebDriver,
  clickableTextXpath,
  pilotFail,
  textXpath,
  xpathLiteral,
} from "./lib/tauri-webdriver.mjs";

const ROOT_DIR = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const PILOT_HOME = path.resolve(process.env.INFIMOUNT_RC10_PILOT_HOME || "");
const PILOT_ROOT = path.resolve(process.env.INFIMOUNT_RC10_PILOT_ROOT || "");
const KIT = path.resolve(process.env.INFIMOUNT_RC10_PILOT_KIT || "");
const EVIDENCE = path.resolve(process.env.INFIMOUNT_RC10_PILOT_EVIDENCE || "");
const APPLICATION = path.resolve(process.env.INFIMOUNT_RC10_PILOT_APP || "");
const DRIVER_URL = process.env.INFIMOUNT_RC10_DRIVER_URL || "http://127.0.0.1:4444";
const CODEX_TIMEOUT_MS = Number.parseInt(
  process.env.INFIMOUNT_CODEX_TIMEOUT_MS || "900000",
  10,
);

const STORAGE_NAME = "RC10 Stable Upgrade Storage";
const STABLE_WORKSPACE_NAME = "Stable retained workspace";
const REAL_WORKSPACE_NAME = "RC10 real pilot";
const REAL_WORKSPACE_ROOT = "/agent-workspaces/rc10-real-pilot";
const CANDIDATE_VERSION = "0.8.1-rc.10";
const CANDIDATE_COMMIT = "86859130f93757d6893bc128e3955906a4f0130e";
const KIT_SCRIPT = path.join(ROOT_DIR, "scripts", "agent-task-pilot-kit.mjs");

const TASKS = [
  {
    klass: "coding",
    folder: "coding",
    objective:
      "Diagnose why the invoice total is too low when a customer has more than one invoice. " +
      "Produce outputs/review.md explaining the root cause and outputs/patch.diff containing a minimal fix. " +
      "Preserve duplicate-invoice-id protection. Do not modify inputs.",
    outputs: ["review.md", "patch.diff"],
    publish: ["review.md"],
    destination: "pilot-kit/coding/publish-destination",
    finalConflictPolicy: "fail",
    staleProbe: true,
    conflictProbe: false,
  },
  {
    klass: "document",
    folder: "document",
    objective:
      "Synthesize these notes into outputs/summary.md and outputs/action-items.md. " +
      "Reconcile repeated information, preserve dates/owners/quantities exactly, call out the one unresolved " +
      "launch dependency, and do not invent decisions. Do not modify inputs.",
    outputs: ["summary.md", "action-items.md"],
    publish: ["summary.md"],
    destination: "pilot-kit/document/publish-destination",
    finalConflictPolicy: "rename",
    staleProbe: false,
    conflictProbe: true,
  },
  {
    klass: "data-analysis",
    folder: "data",
    objective:
      "Analyze orders.csv. Produce outputs/findings.md and outputs/summary.csv. summary.csv must use columns " +
      "metric,value and include exactly these metrics: gross_revenue, net_revenue, refund_amount, refunded_orders, " +
      "order_count, refund_rate_pct, top_region_by_net_revenue, top_product_by_net_revenue. Treat refunded orders " +
      "as zero net revenue. Round refund_rate_pct to two decimals. Do not modify inputs.",
    outputs: ["findings.md", "summary.csv"],
    publish: ["findings.md", "summary.csv"],
    destination: "pilot-kit/data/publish-destination",
    finalConflictPolicy: "fail",
    staleProbe: false,
    conflictProbe: false,
  },
];

function requireFile(file, label) {
  if (!file || !fs.existsSync(file) || !fs.statSync(file).isFile()) {
    pilotFail(label + " is missing: " + file);
  }
}

function requireDir(dir, label) {
  if (!dir || !fs.existsSync(dir) || !fs.statSync(dir).isDirectory()) {
    pilotFail(label + " is missing: " + dir);
  }
}

function readJson(file) {
  return JSON.parse(fs.readFileSync(file, "utf8"));
}

function writeJson(file, value) {
  fs.mkdirSync(path.dirname(file), { recursive: true, mode: 0o700 });
  fs.writeFileSync(file, JSON.stringify(value, null, 2) + "\n", { mode: 0o600 });
}

function sha256File(file) {
  return crypto.createHash("sha256").update(fs.readFileSync(file)).digest("hex");
}

function digestSource(dir) {
  const result = execFileSync(
    process.execPath,
    [KIT_SCRIPT, "digest", dir],
    { cwd: ROOT_DIR, encoding: "utf8" },
  );
  return JSON.parse(result);
}

function checkTask(klass, outputsDir) {
  const result = spawnSync(
    process.execPath,
    [KIT_SCRIPT, "check", klass, "--kit", KIT, "--outputs", outputsDir],
    { cwd: ROOT_DIR, encoding: "utf8" },
  );
  return {
    ok: result.status === 0,
    detail: (result.stderr || result.stdout || "").trim(),
  };
}

function outputSignature(spec, outputsDir) {
  const hash = crypto.createHash("sha256");
  for (const name of [...spec.outputs].sort()) {
    const file = path.join(outputsDir, name);
    hash.update(name);
    hash.update("\0");
    hash.update(fs.readFileSync(file));
    hash.update("\0");
  }
  return hash.digest("hex");
}

function sourceExposed() {
  const storages = readJson(path.join(PILOT_HOME, ".infimount", "storages.json"));
  const storage = storages.find((item) => item.name === STORAGE_NAME);
  if (!storage) pilotFail("pilot source storage is missing");
  return storage.mcp_exposed === true;
}

function workspaceRecord(name) {
  const doc = readJson(path.join(PILOT_HOME, ".infimount", "workspaces.json"));
  const workspace = (doc.workspaces || []).find((item) => item.name === name);
  if (!workspace) pilotFail("workspace is missing: " + name);
  return workspace;
}

async function waitForDriver(processHandle) {
  const deadline = Date.now() + 15000;
  while (Date.now() < deadline) {
    if (processHandle.exitCode !== null) {
      pilotFail("tauri-driver exited early with status " + processHandle.exitCode);
    }
    try {
      const response = await fetch(DRIVER_URL + "/status");
      if (response.ok) return;
    } catch {
      // continue polling
    }
    await sleep(200);
  }
  pilotFail("tauri-driver did not become ready");
}

async function closeDialogContaining(driver, text) {
  const dialogXpath =
    "//*[@role='dialog' and contains(normalize-space(.), " + xpathLiteral(text) + ")]";
  await driver.waitElement("xpath", dialogXpath);
  const closeXpath =
    dialogXpath + "//button[.//span[normalize-space(.)='Close'] or normalize-space(.)='Close']";
  await driver.clickLocated("xpath", closeXpath, 15000);
  await driver.waitAbsent("xpath", dialogXpath, 10000);
}

async function navigateStoragePath(driver, value) {
  await driver.clickCss('button[title="Click to edit path"]');
  const inputXpath = "//form//input";
  await driver.setInputXpath(inputXpath, value);
  const input = await driver.waitElement("xpath", inputXpath);
  await driver.sendKeys(input, "\uE007");
  await driver.waitFor(
    async () => {
      const button = await driver.find(
        "css selector",
        'button[title="Click to edit path"]',
        true,
      );
      if (!button) return false;
      return (await driver.text(button)).trim() === value;
    },
    15000,
    "storage path " + value,
  );
}

async function assertOperational(driver) {
  await driver.waitFor(
    async () => {
      const snapshot = await driver.bodySnapshot(10000);
      if (snapshot.includes("Storage and MCP access are disabled")) {
        pilotFail("rc.10 entered restricted recovery mode: " + snapshot);
      }
      return Boolean(
        await driver.find(
          "css selector",
          '[role="button"][aria-label="' + STORAGE_NAME + '"]',
          true,
        ),
      );
    },
    25000,
    "retained stable storage",
  );
}

async function openWorkspaceAndConnect(driver, workspaceName) {
  await driver.clickCss('button[aria-label="Storage actions"]');
  await driver.clickText("Agent Workspaces");
  await driver.waitText(workspaceName, 20000);

  const workspaceButtonXpath =
    "//button[.//span[normalize-space(.)=" + xpathLiteral(workspaceName) + "]]";
  await driver.clickLocated("xpath", workspaceButtonXpath, 20000);
  await driver.clickText("Connect agent", 20000);
  await driver.waitText("Connect an AI client to one scoped workspace");
  await driver.assertSelectPreselected(
    '[aria-label="Agent Access workspace"]',
    workspaceName,
  );
}

async function prepareRetainedSafetyWorkspace(driver) {
  const stableRoot = path.join(PILOT_HOME, "stable-workspace");
  fs.mkdirSync(stableRoot, { recursive: true });
  fs.writeFileSync(path.join(stableRoot, "sample.txt"), "retained stable safety fixture\n");
  fs.mkdirSync(path.join(PILOT_HOME, "outside"), { recursive: true });
  fs.writeFileSync(path.join(PILOT_HOME, "outside", "denied.txt"), "denied fixture\n");

  await openWorkspaceAndConnect(driver, STABLE_WORKSPACE_NAME);
  await driver.clickAnyText(["Prepare agent access", "Re-check agent access"], 30000);
  await driver.waitText("stdio is on demand", 30000);
  await driver.clickText("Run safety probe", 30000);
  await driver.waitText("Safety probe passed.", 45000);
  await closeDialogContaining(driver, "Connect an AI client to one scoped workspace");
  console.log("retained_workspace_safety_probe=passed");
}

async function createRealWorkspace(driver) {
  await assertOperational(driver);
  await driver.clickCss('button[aria-label="Storage actions"]');
  await driver.clickText("Agent Workspaces");
  await driver.waitText(STABLE_WORKSPACE_NAME, 20000);
  await driver.waitText("Create workspace");

  const existingWorkspaceXpath =
    "//button[.//span[normalize-space(.)=" + xpathLiteral(REAL_WORKSPACE_NAME) + "]]";
  const existing = await driver.find("xpath", existingWorkspaceXpath, true);
  if (!existing) {
    await driver.setInput("#workspace-name", REAL_WORKSPACE_NAME);
    const writeSwitch = await driver.waitElement(
      "css selector",
      'button[aria-label="Allow agent writes"]',
    );
    if ((await driver.attribute(writeSwitch, "data-state")) !== "checked") {
      await driver.clickElement(writeSwitch);
    }
    await driver.clickText("Create workspace", 30000);
    await driver.waitText(REAL_WORKSPACE_NAME, 30000);
  }

  const workspaceButton = await driver.waitElement("xpath", existingWorkspaceXpath, 20000);
  await driver.clickElement(workspaceButton);
  await driver.clickText("Connect agent", 20000);
  await driver.waitText("Connect an AI client to one scoped workspace");
  await driver.assertSelectPreselected(
    '[aria-label="Agent Access workspace"]',
    REAL_WORKSPACE_NAME,
  );
  await driver.clickAnyText(["Prepare agent access", "Re-check agent access"], 30000);
  await driver.waitText("stdio is on demand", 30000);
  await closeDialogContaining(driver, "Connect an AI client to one scoped workspace");

  const workspace = workspaceRecord(REAL_WORKSPACE_NAME);
  if (workspace.accessProfile !== "read_write") {
    pilotFail("real pilot workspace is not read-write");
  }
  if (workspace.rootPath !== REAL_WORKSPACE_ROOT) {
    pilotFail("unexpected real pilot workspace root: " + workspace.rootPath);
  }
  console.log("real_pilot_workspace_ready=yes");
  console.log("upgrade_stable_workspace_visible=yes");
}

async function waitForValidatedOutputs(spec, outputsDir) {
  const deadline = Date.now() + CODEX_TIMEOUT_MS;
  let lastDetail = "outputs not created yet";
  while (Date.now() < deadline) {
    const allPresent = spec.outputs.every((name) => {
      const file = path.join(outputsDir, name);
      return fs.existsSync(file) && fs.statSync(file).isFile() && fs.statSync(file).size > 0;
    });
    if (allPresent) {
      const check = checkTask(spec.klass, outputsDir);
      if (check.ok) {
        const before = outputSignature(spec, outputsDir);
        await sleep(5000);
        const stillPresent = spec.outputs.every((name) => {
          const file = path.join(outputsDir, name);
          return fs.existsSync(file) && fs.statSync(file).isFile() && fs.statSync(file).size > 0;
        });
        if (
          stillPresent &&
          outputSignature(spec, outputsDir) === before &&
          checkTask(spec.klass, outputsDir).ok
        ) {
          return;
        }
      }
      lastDetail = check.detail || "workload validator has not passed or settled yet";
    }
    await sleep(5000);
  }
  pilotFail(spec.klass + " Codex outputs did not pass validation: " + lastDetail);
}

async function reviewOutputs(driver, spec, taskRoot, outputsDir) {
  await driver.clickText("Review outputs", 30000);
  const section = await driver.waitElement(
    "css selector",
    '[data-testid="agent-task-output-review"]',
    30000,
  );
  const text = await driver.text(section);
  if (!text.includes(String(spec.outputs.length) + " output file")) {
    pilotFail(spec.klass + " output review file count is unexpected: " + text);
  }
  await driver.waitText("0 of " + spec.outputs.length + " selected", 30000);
  const bytes = spec.outputs.reduce(
    (sum, name) => sum + fs.statSync(path.join(outputsDir, name)).size,
    0,
  );
  await driver.screenshot(path.join(EVIDENCE, spec.klass, "review.png"));
  return {
    fileCount: spec.outputs.length,
    totalBytes: bytes,
    nothingSelectedByDefault: true,
    taskRoot,
  };
}

async function selectPublicationOutputs(driver, names) {
  for (const name of names) {
    await driver.clickCss('button[aria-label="Publish outputs/' + name + '"]', 20000);
  }
}

async function configurePublication(driver, spec) {
  await driver.assertSelectPreselected(
    '[aria-label="Publication destination storage"]',
    STORAGE_NAME,
  );
  await driver.setInput("#agent-task-publication-dir", spec.destination);
  await driver.assertNoSelectOption(
    '[aria-label="Publication conflict policy"]',
    "overwrite",
    "Fail",
  );
}

async function waitPublicationPreview(driver) {
  return await driver.waitElement(
    "css selector",
    '[data-testid="agent-task-publication-preview"]',
    30000,
  );
}

function receiptForTask(hostTaskRoot) {
  const receipts = fs.readdirSync(hostTaskRoot)
    .filter((name) => /^publish-receipt-[0-9a-f-]+\.json$/i.test(name))
    .sort();
  if (receipts.length !== 1) {
    pilotFail("expected exactly one publication receipt, got " + receipts.length);
  }
  return receipts[0];
}

function verifyPublished(spec, outputsDir) {
  const destinationRoot = path.join(PILOT_HOME, spec.destination);
  if (spec.klass === "coding") {
    const target = path.join(destinationRoot, "review.md");
    if (!fs.existsSync(target) ||
        !fs.readFileSync(target).equals(fs.readFileSync(path.join(outputsDir, "review.md")))) {
      pilotFail("coding destination bytes do not match reviewed output");
    }
  } else if (spec.klass === "document") {
    const sentinel = path.join(destinationRoot, "summary.md");
    if (
      fs.readFileSync(sentinel, "utf8") !==
      "Existing destination sentinel. This file must never be overwritten by the pilot.\n"
    ) {
      pilotFail("document conflict probe overwrote the existing destination");
    }
    const renamed = path.join(destinationRoot, "summary copy.md");
    if (!fs.existsSync(renamed) ||
        !fs.readFileSync(renamed).equals(fs.readFileSync(path.join(outputsDir, "summary.md")))) {
      pilotFail("document renamed destination bytes do not match reviewed output");
    }
  } else {
    for (const name of spec.publish) {
      const target = path.join(destinationRoot, name);
      if (!fs.existsSync(target) ||
          !fs.readFileSync(target).equals(fs.readFileSync(path.join(outputsDir, name)))) {
        pilotFail("data-analysis destination bytes do not match " + name);
      }
    }
  }
}

async function publishTask(driver, spec, hostTaskRoot, outputsDir) {
  await selectPublicationOutputs(driver, spec.publish);
  await configurePublication(driver, spec);

  let staleRejected = false;
  let conflictRejected = false;

  if (spec.conflictProbe) {
    await driver.clickText("Review publication", 30000);
    const preview = await waitPublicationPreview(driver);
    const previewText = await driver.text(preview);
    if (!/Conflicts\s*1/i.test(previewText)) {
      pilotFail("document fail-conflict preview did not report one conflict: " + previewText);
    }
    const publishButton = await driver.find(
      "xpath",
      "//button[contains(normalize-space(.), 'Publish 1 approved output')]",
      true,
    );
    if (publishButton) {
      pilotFail("fail-conflict preview unexpectedly exposed publication");
    }
    conflictRejected = true;
    await driver.selectOption(
      '[aria-label="Publication conflict policy"]',
      "Keep both",
    );
    await driver.clickText("Review publication", 30000);
  } else {
    await driver.clickText("Review publication", 30000);
  }

  await waitPublicationPreview(driver);

  if (spec.staleProbe) {
    const selected = path.join(outputsDir, spec.publish[0]);
    const original = fs.readFileSync(selected);
    fs.appendFileSync(selected, "\n<!-- rc10 stale-preview probe -->\n");
    await driver.clickText(
      "Publish " + spec.publish.length + " approved output" +
        (spec.publish.length === 1 ? "" : "s"),
      30000,
    );
    await driver.waitText("changed since it was reviewed", 30000);
    staleRejected = true;
    fs.writeFileSync(selected, original);

    await driver.clickText("Refresh outputs", 30000);
    await driver.waitText("0 of " + spec.outputs.length + " selected", 30000);
    await selectPublicationOutputs(driver, spec.publish);
    await driver.assertSelectPreselected(
      '[aria-label="Publication destination storage"]',
      STORAGE_NAME,
    );
    await driver.setInput("#agent-task-publication-dir", spec.destination);
    await driver.clickText("Review publication", 30000);
    await waitPublicationPreview(driver);
  }

  await driver.clickText(
    "Publish " + spec.publish.length + " approved output" +
      (spec.publish.length === 1 ? "" : "s"),
    30000,
  );
  const success = await driver.waitElement(
    "css selector",
    '[data-testid="agent-task-publication-success"]',
    30000,
  );
  if (!(await driver.text(success)).includes("Receipt:")) {
    pilotFail(spec.klass + " publication did not expose a receipt");
  }

  verifyPublished(spec, outputsDir);
  const receiptName = receiptForTask(hostTaskRoot);
  const receiptFile = path.join(hostTaskRoot, receiptName);
  await driver.screenshot(path.join(EVIDENCE, spec.klass, "published.png"));

  return {
    selectedCount: spec.publish.length,
    conflictPolicy: spec.finalConflictPolicy,
    overwriteAvailable: false,
    previewApproved: true,
    destinationVerified: true,
    receiptName,
    receiptSha256: sha256File(receiptFile),
    staleRejected,
    conflictRejected,
  };
}

async function runTask(driver, spec) {
  console.log("real_task=" + spec.klass);
  const sourceDir = path.join(KIT, spec.folder, "source");
  const beforeDigest = digestSource(sourceDir);
  const exposedBefore = sourceExposed();

  await navigateStoragePath(driver, "/pilot-kit/" + spec.folder);
  const sourceCard = await driver.waitElement(
    "css selector",
    '[role="option"][aria-label="source"]',
    20000,
  );
  await driver.clickElement(sourceCard);
  await driver.clickText("Use with agent");
  await driver.waitText("Prepare Agent Task");

  await driver.setInput("#agent-task-objective", spec.objective);
  await driver.setInput("#agent-task-outputs", spec.outputs.join("\n"));
  await driver.assertSelectPreselected(
    '[aria-label="Local Agent Workspace"]',
    REAL_WORKSPACE_NAME,
  );

  await driver.clickText("Review scope", 30000);
  const preflight = await driver.waitElement(
    "css selector",
    '[data-testid="agent-task-preflight"]',
    30000,
  );
  const preflightText = await driver.text(preflight);
  if (!preflightText.includes("Selected") ||
      !preflightText.includes(String(beforeDigest.fileCount))) {
    pilotFail(spec.klass + " preflight does not match canonical source scope: " + preflightText);
  }

  await driver.clickText("Prepare task", 30000);
  await driver.waitElement("css selector", '[data-testid="agent-task-prepared"]', 30000);
  const taskRootElement = await driver.waitElement(
    "xpath",
    "//*[@data-testid='agent-task-prepared']//code[contains(normalize-space(.), 'tasks/')]",
    30000,
  );
  const taskRoot = (await driver.text(taskRootElement)).trim().replace(/^\/+/, "");
  if (!/^tasks\/[0-9a-f-]{36}$/i.test(taskRoot)) {
    pilotFail("unexpected prepared task root: " + taskRoot);
  }
  const taskId = taskRoot.split("/").at(-1);
  const hostTaskRoot = path.join(
    PILOT_HOME,
    REAL_WORKSPACE_ROOT.replace(/^\/+/, ""),
    taskRoot,
  );
  const outputsDir = path.join(hostTaskRoot, "outputs");

  await driver.clickText("Open in Codex", 30000);
  await driver.waitText("Codex handoff opened.", 30000);
  console.log("codex_handoff=" + spec.klass + ":launched");

  await waitForValidatedOutputs(spec, outputsDir);
  console.log("codex_outputs=" + spec.klass + ":validated");

  const review = await reviewOutputs(driver, spec, taskRoot, outputsDir);
  const publication = await publishTask(driver, spec, hostTaskRoot, outputsDir);
  const afterDigest = digestSource(sourceDir);
  const exposedAfter = sourceExposed();

  if (beforeDigest.digest !== afterDigest.digest) {
    pilotFail(spec.klass + " source digest changed");
  }
  if (exposedBefore !== exposedAfter) {
    pilotFail(spec.klass + " source MCP exposure changed");
  }

  await driver.clickText("Close", 15000);
  await driver.waitAbsent("xpath", textXpath("Prepare Agent Task"), 10000);

  return {
    class: spec.klass,
    taskId,
    sourceStorageKind: "local",
    workspaceStorageKind: "local",
    sourceMcpExposedBefore: exposedBefore,
    sourceMcpExposedAfter: exposedAfter,
    sourceSelectionDigestBefore: beforeDigest.digest,
    sourceSelectionDigestAfter: afterDigest.digest,
    preflight: {
      selectedItems: 1,
      fileCount: beforeDigest.fileCount,
      totalBytes: beforeDigest.totalBytes,
    },
    review: {
      fileCount: review.fileCount,
      totalBytes: review.totalBytes,
      nothingSelectedByDefault: true,
    },
    publication: {
      selectedCount: publication.selectedCount,
      conflictPolicy: publication.conflictPolicy,
      overwriteAvailable: false,
      previewApproved: true,
      destinationVerified: true,
      receiptPath: taskRoot + "/" + publication.receiptName,
      receiptSha256: publication.receiptSha256,
    },
    uiEvidence: [
      spec.klass + "/review.png",
      spec.klass + "/published.png",
    ],
    passed: false,
    automated: {
      outputValidatorPassed: true,
      stalePreviewRejected: publication.staleRejected,
      failConflictRejected: publication.conflictRejected,
    },
  };
}

async function main() {
  for (const [value, label] of [
    [PILOT_HOME, "pilot HOME"],
    [PILOT_ROOT, "pilot root"],
    [KIT, "pilot kit"],
    [EVIDENCE, "evidence directory"],
  ]) requireDir(value, label);
  requireFile(APPLICATION, "installed rc.10 application");
  requireFile(KIT_SCRIPT, "Agent Task pilot kit");

  const upgrade = readJson(path.join(EVIDENCE, "upgrade.json"));
  if (!upgrade.passed || upgrade.from !== "0.8.0" || upgrade.to !== CANDIDATE_VERSION) {
    pilotFail("stable upgrade evidence is missing or incomplete");
  }

  fs.mkdirSync(path.join(KIT, "coding", "publish-destination"), { recursive: true });
  fs.mkdirSync(path.join(KIT, "data", "publish-destination"), { recursive: true });

  const driverProcess = spawn("tauri-driver", [], {
    cwd: ROOT_DIR,
    env: process.env,
    stdio: ["ignore", "pipe", "pipe"],
  });
  let driverLog = "";
  driverProcess.stdout.on("data", (chunk) => {
    driverLog = (driverLog + chunk.toString()).slice(-30000);
  });
  driverProcess.stderr.on("data", (chunk) => {
    driverLog = (driverLog + chunk.toString()).slice(-30000);
  });

  const driver = new TauriWebDriver(DRIVER_URL);
  try {
    await waitForDriver(driverProcess);
    await driver.createSession(APPLICATION);
    await assertOperational(driver);
    await prepareRetainedSafetyWorkspace(driver);
    await createRealWorkspace(driver);

    const taskResults = [];
    for (const spec of TASKS) {
      taskResults.push(await runTask(driver, spec));
    }

    const stalePreviewRejected = taskResults.some(
      (task) => task.automated.stalePreviewRejected,
    );
    const failConflictRejected = taskResults.some(
      (task) => task.automated.failConflictRejected,
    );
    if (!stalePreviewRejected || !failConflictRejected) {
      pilotFail("mandatory safety probes were not observed");
    }

    const template = readJson(path.join(KIT, "pilot-evidence.template.json"));
    template.synthetic = false;
    template.candidate.version = CANDIDATE_VERSION;
    template.candidate.commit = CANDIDATE_COMMIT;
    template.candidate.platform = "linux";
    template.candidate.installedFrom = "0.8.0";
    template.client = { name: "codex", handoffViaInfimountMcp: true };
    template.tasks = taskResults.map(({ automated, ...task }) => task);
    template.safetyProbes = {
      failConflictRejected: true,
      stalePreviewRejected: true,
      overwriteUnavailable: true,
    };
    template.upgrade = {
      from: "0.8.0",
      to: CANDIDATE_VERSION,
      configurationRetained: true,
      storageRegistryRetained: true,
      workspaceRegistryRetained: true,
      agentTasksVisible: true,
      passed: true,
    };
    template.overallPassed = false;
    template.observations = [
      "All deterministic workload validators passed.",
      "All three tasks were handed to Codex through Infimount task-scoped MCP.",
      "Automated safety and publication checks passed; final human usefulness acceptance is pending.",
    ];

    writeJson(path.join(EVIDENCE, "pilot-evidence.pending.json"), template);
    writeJson(path.join(EVIDENCE, "automated-results.json"), {
      candidate: CANDIDATE_VERSION,
      candidateCommit: CANDIDATE_COMMIT,
      allAutomatedChecksPassed: true,
      taskClasses: TASKS.map((task) => task.klass),
      safetyProbes: template.safetyProbes,
      upgrade: template.upgrade,
      qualityAcceptancePending: true,
    });

    fs.writeFileSync(
      path.join(EVIDENCE, "QUALITY-REVIEW.md"),
      [
        "# RC10 real Agent Task quality review",
        "",
        "The automated runner has already verified:",
        "- coding patch applies and the canonical tests pass;",
        "- document outputs contain every required source fact;",
        "- data-analysis metrics exactly match the independent calculation;",
        "- source digests and source MCP exposure are unchanged;",
        "- publication safety probes and destination-byte checks passed.",
        "",
        "Review the private screenshots and generated outputs under this pilot root.",
        "If the three outputs are genuinely useful and acceptable, run:",
        "",
        "  node scripts/rc10-final-pilot-accept.mjs --root <PILOT_ROOT> --accept-quality",
        "",
        "Do not accept merely because the automated checks passed.",
        "",
      ].join("\n"),
      { mode: 0o600 },
    );

    console.log("real_codex_workloads_automated=yes");
    console.log("real_pilot_pending_evidence=" + path.join(EVIDENCE, "pilot-evidence.pending.json"));
    console.log("quality_acceptance_pending=yes");
  } catch (error) {
    console.error(String(error && error.stack || error));
    if (driverLog.trim()) {
      console.error("===== tauri-driver log tail =====");
      console.error(driverLog);
    }
    process.exitCode = 1;
  } finally {
    await driver.quit();
    if (driverProcess.exitCode === null) {
      driverProcess.kill("SIGTERM");
      await sleep(500);
      if (driverProcess.exitCode === null) driverProcess.kill("SIGKILL");
    }
  }
}

await main();
