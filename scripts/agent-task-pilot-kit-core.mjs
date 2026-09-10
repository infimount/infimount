#!/usr/bin/env node
import crypto from "node:crypto";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { execFileSync } from "node:child_process";

const CANDIDATE_VERSION = "0.8.1-rc.1";
const CANDIDATE_COMMIT = "18399d3b895551ed4f94bccdcc119f16a53889fc";
const INSTALLED_FROM = "0.8.0";

const ORDER_ROWS = [
  ["O001", "South", "web", "Alpha", 2, 1200, 0], ["O002", "West", "store", "Beta", 1, 1800, 0],
  ["O003", "North", "web", "Alpha", 1, 1200, 1], ["O004", "East", "partner", "Gamma", 3, 700, 0],
  ["O005", "South", "store", "Beta", 2, 1800, 0], ["O006", "West", "web", "Gamma", 4, 700, 0],
  ["O007", "North", "store", "Delta", 1, 2500, 0], ["O008", "East", "web", "Alpha", 2, 1200, 0],
  ["O009", "South", "partner", "Gamma", 2, 700, 1], ["O010", "West", "store", "Delta", 2, 2500, 0],
  ["O011", "North", "web", "Beta", 3, 1800, 0], ["O012", "East", "store", "Gamma", 5, 700, 0],
  ["O013", "South", "web", "Delta", 1, 2500, 0], ["O014", "West", "partner", "Alpha", 3, 1200, 1],
  ["O015", "North", "store", "Gamma", 2, 700, 0], ["O016", "East", "web", "Beta", 2, 1800, 0],
  ["O017", "South", "store", "Alpha", 4, 1200, 0], ["O018", "West", "web", "Beta", 1, 1800, 0],
  ["O019", "North", "partner", "Delta", 1, 2500, 0], ["O020", "East", "store", "Alpha", 1, 1200, 1],
  ["O021", "South", "web", "Gamma", 6, 700, 0], ["O022", "West", "store", "Beta", 2, 1800, 0],
  ["O023", "North", "web", "Alpha", 3, 1200, 0], ["O024", "East", "partner", "Delta", 1, 2500, 0],
];

const fail = (message) => {
  console.error(`Agent Task pilot kit failed: ${message}`);
  process.exit(1);
};

const pilotPlatform = () => {
  const value = { linux: "linux", darwin: "macos", win32: "windows" }[process.platform];
  if (!value) fail(`unsupported pilot platform ${process.platform}`);
  return value;
};

const normalizeRel = (value) => value.split(path.sep).join("/");

const ensureEmptyDir = (dir) => {
  if (fs.existsSync(dir)) {
    if (fs.readdirSync(dir).length > 0) fail(`refusing to overwrite non-empty directory ${dir}`);
  } else {
    fs.mkdirSync(dir, { recursive: true });
  }
};

const write = (root, rel, content) => {
  const target = path.join(root, rel);
  fs.mkdirSync(path.dirname(target), { recursive: true });
  fs.writeFileSync(target, content, "utf8");
};

const walkFiles = (root) => {
  const files = [];
  const visit = (dir) => {
    for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
      const full = path.join(dir, entry.name);
      if (entry.isDirectory()) visit(full);
      else if (entry.isFile()) files.push(full);
    }
  };
  visit(root);
  return files.sort((a, b) => normalizeRel(path.relative(root, a)).localeCompare(normalizeRel(path.relative(root, b))));
};

const digestTree = (root) => {
  if (!fs.existsSync(root) || !fs.statSync(root).isDirectory()) fail(`digest root is not a directory: ${root}`);
  const files = walkFiles(root);
  if (files.length === 0) fail(`digest root contains no files: ${root}`);
  const aggregate = crypto.createHash("sha256");
  let totalBytes = 0;
  for (const file of files) {
    const bytes = fs.readFileSync(file);
    const rel = normalizeRel(path.relative(root, file));
    const fileSha = crypto.createHash("sha256").update(bytes).digest("hex");
    totalBytes += bytes.length;
    aggregate.update(`${rel}\0${bytes.length}\0${fileSha}\n`, "utf8");
  }
  return { digest: aggregate.digest("hex"), fileCount: files.length, totalBytes };
};

const makeOrdersCsv = () => {
  const header = ["order_id", "region", "channel", "product", "quantity", "unit_price", "refunded"];
  return [header, ...ORDER_ROWS].map((row) => row.join(",")).join("\n") + "\n";
};

const topKey = (values) => [...values.entries()].sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0]))[0][0];

const calculateExpectedDataMetrics = () => {
  let gross = 0;
  let refunds = 0;
  let refundedOrders = 0;
  const regionNet = new Map();
  const productNet = new Map();

  for (const [, region, , product, quantity, unitPrice, refunded] of ORDER_ROWS) {
    const value = quantity * unitPrice;
    gross += value;
    if (refunded) {
      refunds += value;
      refundedOrders += 1;
    }
    const net = refunded ? 0 : value;
    regionNet.set(region, (regionNet.get(region) || 0) + net);
    productNet.set(product, (productNet.get(product) || 0) + net);
  }

  return {
    gross_revenue: String(gross),
    net_revenue: String(gross - refunds),
    refund_amount: String(refunds),
    refunded_orders: String(refundedOrders),
    order_count: String(ORDER_ROWS.length),
    refund_rate_pct: ((refundedOrders / ORDER_ROWS.length) * 100).toFixed(2),
    top_region_by_net_revenue: topKey(regionNet),
    top_product_by_net_revenue: topKey(productNet),
  };
};

const evidenceTemplate = () => ({
  schemaVersion: 1,
  synthetic: false,
  candidate: {
    version: CANDIDATE_VERSION,
    commit: CANDIDATE_COMMIT,
    platform: pilotPlatform(),
    installedFrom: INSTALLED_FROM,
  },
  client: { name: "codex", handoffViaInfimountMcp: null },
  tasks: ["coding", "document", "data-analysis"].map((klass) => ({
    class: klass,
    taskId: "REPLACE_WITH_TASK_UUID",
    sourceStorageKind: "local",
    workspaceStorageKind: "local",
    sourceMcpExposedBefore: null,
    sourceMcpExposedAfter: null,
    sourceSelectionDigestBefore: "REPLACE_WITH_SHA256",
    sourceSelectionDigestAfter: "REPLACE_WITH_SHA256",
    preflight: { selectedItems: 0, fileCount: 0, totalBytes: 0 },
    review: { fileCount: 0, totalBytes: 0, nothingSelectedByDefault: null },
    publication: {
      selectedCount: 0,
      conflictPolicy: null,
      overwriteAvailable: null,
      previewApproved: null,
      destinationVerified: null,
      receiptPath: "REPLACE_WITH_RECEIPT_PATH",
      receiptSha256: "REPLACE_WITH_SHA256",
    },
    uiEvidence: [],
    passed: false,
  })),
  safetyProbes: {
    failConflictRejected: null,
    stalePreviewRejected: null,
    overwriteUnavailable: null,
  },
  upgrade: {
    from: INSTALLED_FROM,
    to: CANDIDATE_VERSION,
    configurationRetained: null,
    storageRegistryRetained: null,
    workspaceRegistryRetained: null,
    agentTasksVisible: null,
    passed: false,
  },
  overallPassed: false,
  observations: [],
});

const assertTemplateStartsUnobserved = (template) => {
  if (template.overallPassed !== false || template.upgrade.passed !== false) fail("evidence template must start unpassed");
  if (template.client.handoffViaInfimountMcp !== null) fail("evidence template must not pre-fill MCP handoff success");
  for (const task of template.tasks) {
    if (
      task.passed !== false ||
      task.sourceMcpExposedBefore !== null ||
      task.review.nothingSelectedByDefault !== null ||
      task.publication.conflictPolicy !== null ||
      task.publication.previewApproved !== null
    ) fail("evidence template must not pre-fill observed task results");
  }
  for (const value of Object.values(template.safetyProbes)) {
    if (value !== null) fail("evidence template must not pre-fill safety probe success");
  }
};

const runbook = (root) => `# Infimount Agent Tasks real pilot kit\n\nCandidate: ${CANDIDATE_VERSION}\nCommit: ${CANDIDATE_COMMIT}\nInstalled-from stable: ${INSTALLED_FROM}\n\nThis directory contains deterministic local workloads, not pilot evidence. The generated evidence template intentionally starts in a failing/unobserved state. Fill observed fields only after the real desktop/Codex flow.\n\n## 0. Upgrade exercise\n\n1. Start from an installed Infimount ${INSTALLED_FROM} environment with representative configuration, at least one storage registration, and at least one Agent Workspace.\n2. Install ${CANDIDATE_VERSION} over that installation. This validates installer-over-install only, not stable-channel updater behavior.\n3. Confirm the application starts, prior configuration/storage/workspace entries remain, and Agent Tasks are visible.\n\n## 1. Coding pilot\n\nSelect only \`coding/source/\`. Before preparation:\n\n\`node scripts/agent-task-pilot-kit.mjs digest ${normalizeRel(path.join(root, "coding/source"))}\`\n\nCodex prompt:\n\n> Diagnose why the invoice total is too low when a customer has more than one invoice. Produce outputs/review.md explaining the root cause and outputs/patch.diff containing a minimal fix. Preserve duplicate-invoice-id protection. Do not modify inputs.\n\nCheck the result independently:\n\n\`node scripts/agent-task-pilot-kit.mjs check coding --kit ${normalizeRel(root)} --outputs <TASK_OUTPUTS_DIR>\`\n\nUse this task for the stale-preview probe: obtain and approve a publication preview, change the reviewed output bytes, attempt the stale preview and require rejection, then re-review/re-preview before successful publication. Re-run the source digest afterward and require an exact match.\n\n## 2. Document pilot\n\nSelect only \`document/source/\`.\n\nPrompt:\n\n> Synthesize these notes into outputs/summary.md and outputs/action-items.md. Reconcile repeated information, preserve dates/owners/quantities exactly, call out the one unresolved launch dependency, and do not invent decisions. Do not modify inputs.\n\nCheck:\n\n\`node scripts/agent-task-pilot-kit.mjs check document --kit ${normalizeRel(root)} --outputs <TASK_OUTPUTS_DIR>\`\n\nPublication probe: \`document/publish-destination/summary.md\` already exists. First use conflict policy **fail** and verify rejection. Then choose **rename**, review the rename plan, approve it, and publish. Verify no overwrite mode exists. Re-run the source digest afterward and require an exact match.\n\n## 3. Data-analysis pilot\n\nSelect only \`data/source/\`.\n\nPrompt:\n\n> Analyze orders.csv. Produce outputs/findings.md and outputs/summary.csv. summary.csv must use columns metric,value and include exactly these metrics: gross_revenue, net_revenue, refund_amount, refunded_orders, order_count, refund_rate_pct, top_region_by_net_revenue, top_product_by_net_revenue. Treat refunded orders as zero net revenue. Round refund_rate_pct to two decimals. Do not modify inputs.\n\nCheck:\n\n\`node scripts/agent-task-pilot-kit.mjs check data-analysis --kit ${normalizeRel(root)} --outputs <TASK_OUTPUTS_DIR>\`\n\nRe-run the source digest afterward and require an exact match.\n\n## Evidence\n\nCapture at least two screenshots per task in a private local evidence directory using relative names such as \`coding/review.png\` and \`coding/published.png\`. Never record source/host paths, source content, storage IDs/configuration, credentials, OAuth values, or secrets in the evidence JSON.\n\nCopy and fill \`pilot-evidence.template.json\`, then validate it from a checkout containing the candidate tag:\n\n\`node scripts/check-agent-task-pilot-evidence.mjs /path/to/pilot-evidence.json\`\n\nA checker pass is necessary but not sufficient. Mark a task passed only when its output is genuinely useful/correct and the real safety flow was acceptable.\n`;

const prepare = (out) => {
  const root = path.resolve(out);
  ensureEmptyDir(root);

  write(root, "coding/source/package.json", JSON.stringify({ type: "module", scripts: { test: "node --test" } }, null, 2) + "\n");
  write(root, "coding/source/src/invoiceTotals.mjs", `export function totalOpenInvoices(invoices) {\n  const seenCustomers = new Set();\n  let total = 0;\n\n  for (const invoice of invoices) {\n    if (invoice.status !== "open") continue;\n    if (seenCustomers.has(invoice.customerId)) continue;\n    seenCustomers.add(invoice.customerId);\n    total += invoice.amount;\n  }\n\n  return total;\n}\n`);
  write(root, "coding/source/test/invoiceTotals.test.mjs", `import test from "node:test";\nimport assert from "node:assert/strict";\nimport { totalOpenInvoices } from "../src/invoiceTotals.mjs";\n\ntest("counts multiple open invoices for the same customer", () => {\n  const invoices = [\n    { id: "I-100", customerId: "C-1", amount: 1200, status: "open" },\n    { id: "I-101", customerId: "C-1", amount: 800, status: "open" },\n    { id: "I-102", customerId: "C-2", amount: 500, status: "open" },\n  ];\n  assert.equal(totalOpenInvoices(invoices), 2500);\n});\n\ntest("does not double count a repeated invoice id", () => {\n  const invoices = [\n    { id: "I-200", customerId: "C-1", amount: 900, status: "open" },\n    { id: "I-200", customerId: "C-1", amount: 900, status: "open" },\n    { id: "I-201", customerId: "C-1", amount: 100, status: "open" },\n  ];\n  assert.equal(totalOpenInvoices(invoices), 1000);\n});\n\ntest("ignores closed invoices", () => {\n  assert.equal(totalOpenInvoices([\n    { id: "I-300", customerId: "C-3", amount: 700, status: "closed" },\n    { id: "I-301", customerId: "C-3", amount: 400, status: "open" },\n  ]), 400);\n});\n`);
  write(root, "coding/source/README.md", "# Invoice totals\n\n`totalOpenInvoices` returns the total value of unique open invoices. Repeated records with the same invoice id must be counted only once. A customer may legitimately have multiple distinct open invoices.\n");

  write(root, "document/source/launch-plan.md", "# Launch plan\n\nTarget pilot launch: 14 October. Owner for release coordination: Priya. Initial rollout is limited to 12 customer accounts. The launch cannot proceed until Security signs off on the data-retention wording. Product approved the in-app copy on 3 October.\n");
  write(root, "document/source/ops-review.md", "# Operations review\n\nPriya confirmed the 14 October pilot date is still the target. Support coverage will be 09:00-21:00 IST during the first three days. Mateo owns the rollback checklist and must finish it by 10 October. Security review of data-retention wording is still open; no approval date was committed.\n");
  write(root, "document/source/customer-feedback.md", "# Customer feedback\n\nSeven of the 12 pilot customers asked for a concise admin setup guide. Documentation owner Asha agreed to publish a two-page setup guide by 11 October. Two customers requested SSO changes, but Product explicitly moved SSO expansion out of this pilot. No customer requested a launch-date change.\n");
  write(root, "document/publish-destination/summary.md", "Existing destination sentinel. This file must never be overwritten by the pilot.\n");

  write(root, "data/source/orders.csv", makeOrdersCsv());
  const template = evidenceTemplate();
  assertTemplateStartsUnobserved(template);
  write(root, "pilot-evidence.template.json", JSON.stringify(template, null, 2) + "\n");
  write(root, "RUNBOOK.md", runbook(root));

  console.log(JSON.stringify({
    root,
    candidate: CANDIDATE_VERSION,
    commit: CANDIDATE_COMMIT,
    expectedDataMetrics: calculateExpectedDataMetrics(),
    sourceDigests: {
      coding: digestTree(path.join(root, "coding/source")),
      document: digestTree(path.join(root, "document/source")),
      "data-analysis": digestTree(path.join(root, "data/source")),
    },
  }, null, 2));
};

const parseSummaryCsv = (file) => {
  const lines = fs.readFileSync(file, "utf8").trim().split(/\r?\n/);
  if (lines.length < 2 || lines[0].trim() !== "metric,value") fail("summary.csv must start with metric,value");
  const result = {};
  for (const line of lines.slice(1)) {
    const comma = line.indexOf(",");
    if (comma <= 0) fail(`invalid summary.csv row: ${line}`);
    const key = line.slice(0, comma).trim();
    const value = line.slice(comma + 1).trim().replace(/^"|"$/g, "");
    if (key in result) fail(`duplicate metric in summary.csv: ${key}`);
    result[key] = value;
  }
  return result;
};

const requireOutput = (outputs, rel) => {
  const file = path.join(outputs, rel);
  if (!fs.existsSync(file) || !fs.statSync(file).isFile() || fs.statSync(file).size === 0) fail(`missing or empty output ${rel}`);
  return file;
};

const checkCoding = (kit, outputs) => {
  requireOutput(outputs, "review.md");
  const patchFile = requireOutput(outputs, "patch.diff");
  const work = fs.mkdtempSync(path.join(os.tmpdir(), "infimount-pilot-coding-"));
  fs.cpSync(path.join(kit, "coding/source"), work, { recursive: true });
  try {
    execFileSync("git", ["apply", "--check", patchFile], { cwd: work, stdio: "pipe" });
    execFileSync("git", ["apply", patchFile], { cwd: work, stdio: "pipe" });
    execFileSync(process.execPath, ["--test"], { cwd: work, stdio: "pipe" });
  } catch (error) {
    const detail = error.stderr?.toString().trim() || error.stdout?.toString().trim() || error.message;
    fail(`coding output does not apply cleanly and pass tests: ${detail}`);
  } finally {
    fs.rmSync(work, { recursive: true, force: true });
  }
  console.log("Coding workload check passed: patch applies and all source tests pass.");
};

const requireContains = (text, pattern, label) => {
  if (!pattern.test(text)) fail(`document output is missing ${label}`);
};

const checkDocument = (outputs) => {
  const summary = fs.readFileSync(requireOutput(outputs, "summary.md"), "utf8");
  const actions = fs.readFileSync(requireOutput(outputs, "action-items.md"), "utf8");
  const combined = `${summary}\n${actions}`;
  for (const [pattern, label] of [
    [/14\s+October/i, "the 14 October target date"], [/Priya/i, "release owner Priya"],
    [/12\s+(customer|pilot)/i, "the 12-customer pilot scope"], [/Security/i, "the unresolved Security dependency"],
    [/data[- ]retention/i, "data-retention wording"], [/Mateo/i, "Mateo rollback owner"],
    [/10\s+October/i, "the rollback deadline"], [/Asha/i, "Asha documentation owner"],
    [/11\s+October/i, "the setup-guide deadline"], [/SSO/i, "the explicitly out-of-scope SSO request"],
  ]) requireContains(combined, pattern, label);
  console.log("Document workload check passed: required source facts are represented in the outputs.");
};

const checkData = (outputs) => {
  requireOutput(outputs, "findings.md");
  const actual = parseSummaryCsv(requireOutput(outputs, "summary.csv"));
  const expected = calculateExpectedDataMetrics();
  const actualKeys = Object.keys(actual).sort();
  const expectedKeys = Object.keys(expected).sort();
  if (JSON.stringify(actualKeys) !== JSON.stringify(expectedKeys)) fail(`summary.csv metric set mismatch; expected ${expectedKeys.join(", ")}, got ${actualKeys.join(", ")}`);
  for (const key of expectedKeys) if (actual[key] !== expected[key]) fail(`summary.csv ${key} expected ${expected[key]}, got ${actual[key]}`);
  console.log("Data-analysis workload check passed: all independently computed metrics match.");
};

const check = (klass, kit, outputs) => {
  const kitRoot = path.resolve(kit);
  const outputRoot = path.resolve(outputs);
  if (!fs.existsSync(kitRoot)) fail(`kit directory does not exist: ${kitRoot}`);
  if (!fs.existsSync(outputRoot)) fail(`outputs directory does not exist: ${outputRoot}`);
  if (klass === "coding") return checkCoding(kitRoot, outputRoot);
  if (klass === "document") return checkDocument(outputRoot);
  if (klass === "data-analysis") return checkData(outputRoot);
  fail(`unknown workload class ${klass}`);
};

const selfTest = () => {
  const root = path.join(os.tmpdir(), `infimount-agent-task-pilot-self-test-${process.pid}`);
  try {
    fs.rmSync(root, { recursive: true, force: true });
    prepare(root);
    assertTemplateStartsUnobserved(JSON.parse(fs.readFileSync(path.join(root, "pilot-evidence.template.json"), "utf8")));

    for (const source of ["coding/source", "document/source", "data/source"]) {
      const first = digestTree(path.join(root, source));
      const second = digestTree(path.join(root, source));
      if (JSON.stringify(first) !== JSON.stringify(second)) fail(`${source} digest is not deterministic`);
    }

    const codingOutputs = path.join(root, "self-test-coding-outputs");
    fs.mkdirSync(codingOutputs, { recursive: true });
    write(codingOutputs, "review.md", "Root cause: deduplication uses customerId instead of invoice id.\n");
    write(codingOutputs, "patch.diff", `diff --git a/src/invoiceTotals.mjs b/src/invoiceTotals.mjs\n--- a/src/invoiceTotals.mjs\n+++ b/src/invoiceTotals.mjs\n@@ -1,11 +1,11 @@\n export function totalOpenInvoices(invoices) {\n-  const seenCustomers = new Set();\n+  const seenInvoices = new Set();\n   let total = 0;\n \n   for (const invoice of invoices) {\n     if (invoice.status !== "open") continue;\n-    if (seenCustomers.has(invoice.customerId)) continue;\n-    seenCustomers.add(invoice.customerId);\n+    if (seenInvoices.has(invoice.id)) continue;\n+    seenInvoices.add(invoice.id);\n     total += invoice.amount;\n   }\n \n`);
    checkCoding(root, codingOutputs);

    const documentOutputs = path.join(root, "self-test-document-outputs");
    fs.mkdirSync(documentOutputs, { recursive: true });
    write(documentOutputs, "summary.md", "Pilot remains targeted for 14 October, coordinated by Priya, for 12 customer accounts. Security approval of the data-retention wording remains unresolved. SSO changes are out of scope.\n");
    write(documentOutputs, "action-items.md", "- Mateo: finish rollback checklist by 10 October.\n- Asha: publish setup guide by 11 October.\n");
    checkDocument(documentOutputs);

    const dataOutputs = path.join(root, "self-test-data-outputs");
    fs.mkdirSync(dataOutputs, { recursive: true });
    write(dataOutputs, "findings.md", "Independent fixture totals.\n");
    write(dataOutputs, "summary.csv", `metric,value\n${Object.entries(calculateExpectedDataMetrics()).map(([key, value]) => `${key},${value}`).join("\n")}\n`);
    checkData(dataOutputs);

    console.log("Agent Task pilot kit self-test passed.");
  } finally {
    fs.rmSync(root, { recursive: true, force: true });
  }
};

const getArg = (name) => {
  const index = process.argv.indexOf(name);
  return index >= 0 ? process.argv[index + 1] : undefined;
};

const command = process.argv[2];
if (command === "prepare") {
  prepare(getArg("--out") || path.join(os.tmpdir(), `infimount-agent-task-pilot-${CANDIDATE_VERSION}`));
} else if (command === "digest") {
  if (!process.argv[3]) fail("usage: agent-task-pilot-kit.mjs digest <source-directory>");
  console.log(JSON.stringify(digestTree(path.resolve(process.argv[3])), null, 2));
} else if (command === "check") {
  const klass = process.argv[3];
  const kit = getArg("--kit");
  const outputs = getArg("--outputs");
  if (!klass || !kit || !outputs) fail("usage: agent-task-pilot-kit.mjs check <coding|document|data-analysis> --kit <kit-dir> --outputs <outputs-dir>");
  check(klass, kit, outputs);
} else if (command === "self-test") {
  selfTest();
} else {
  fail("usage: agent-task-pilot-kit.mjs <prepare|digest|check|self-test> ...");
}
