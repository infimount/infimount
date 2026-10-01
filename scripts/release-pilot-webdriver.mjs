#!/usr/bin/env node

import crypto from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import { execFileSync, spawn, spawnSync } from "node:child_process";
import { setTimeout as sleep } from "node:timers/promises";
import { fileURLToPath } from "node:url";

const ELEMENT_KEY = "element-6066-11e4-a52e-4f735466cecf";
const DRIVER_URL = process.env.INFIMOUNT_PILOT_DRIVER_URL || "http://127.0.0.1:4444";
const HOME_DIR = path.resolve(process.env.INFIMOUNT_PILOT_HOME || process.env.HOME || "");
const APP_BINARY = path.resolve(process.env.INFIMOUNT_PILOT_APP || "");
const SIDECAR = path.resolve(process.env.INFIMOUNT_PILOT_SIDECAR || "");
const ROOT_DIR = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const STATE_SCRIPT = path.join(ROOT_DIR, "scripts", "release-pilot-state.mjs");
const PILOT_KIT_CORE = path.join(ROOT_DIR, "scripts", "agent-task-pilot-kit-core.mjs");
const PILOT_KIT = path.resolve(process.env.INFIMOUNT_PILOT_KIT || "");
const STORAGE_ID = "release-pilot-home";
const READ_ONLY_WORKSPACE = "Release pilot workspace";
const READ_ONLY_ROOT = "agent-workspaces/release-pilot-workspace";
const WRITABLE_WORKSPACE = "Release pilot writable";
const WRITABLE_ROOT = "agent-workspaces/release-pilot-writable";

function fail(message) {
  throw new Error(`Release pilot WebDriver failed: ${message}`);
}

function requireFile(file, label) {
  if (!file || !fs.existsSync(file) || !fs.statSync(file).isFile()) {
    fail(`${label} is missing: ${file}`);
  }
}

function requireDirectory(directory, label) {
  if (!directory || !fs.existsSync(directory) || !fs.statSync(directory).isDirectory()) {
    fail(`${label} is missing: ${directory}`);
  }
}

function assert(condition, message) {
  if (!condition) fail(message);
}

function sha256File(file) {
  return crypto.createHash("sha256").update(fs.readFileSync(file)).digest("hex");
}

function treeDigest(root) {
  const files = [];
  const visit = (dir) => {
    for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
      const full = path.join(dir, entry.name);
      if (entry.isDirectory()) visit(full);
      else if (entry.isFile()) files.push(full);
    }
  };
  visit(root);
  files.sort((a, b) => path.relative(root, a).localeCompare(path.relative(root, b)));
  const aggregate = crypto.createHash("sha256");
  let totalBytes = 0;
  for (const file of files) {
    const bytes = fs.readFileSync(file);
    const rel = path.relative(root, file).split(path.sep).join("/");
    const digest = crypto.createHash("sha256").update(bytes).digest("hex");
    totalBytes += bytes.length;
    aggregate.update(`${rel}\0${bytes.length}\0${digest}\n`);
  }
  return { digest: aggregate.digest("hex"), fileCount: files.length, totalBytes };
}

function workspaceRecord(name) {
  const doc = JSON.parse(fs.readFileSync(path.join(HOME_DIR, ".infimount", "workspaces.json"), "utf8"));
  const workspace = (doc.workspaces || []).find((item) => item.name === name);
  if (!workspace) fail(`workspace not found: ${name}`);
  return workspace;
}

function xpathLiteral(value) {
  if (!value.includes("'")) return `'${value}'`;
  if (!value.includes('"')) return `"${value}"`;
  const parts = value.split("'");
  return `concat(${parts.map((part, index) => `${index ? `"'",` : ""}'${part}'`).join(",")})`;
}

function clickableTextXpath(text) {
  return `//*[self::button or @role='menuitem' or @role='option'][contains(normalize-space(.), ${xpathLiteral(text)})]`;
}

function textXpath(text) {
  return `//*[contains(normalize-space(.), ${xpathLiteral(text)})]`;
}

async function rawRequest(method, endpoint, body) {
  const response = await fetch(`${DRIVER_URL}${endpoint}`, {
    method,
    headers: body === undefined ? undefined : { "content-type": "application/json" },
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  const raw = await response.text();
  let payload = {};
  if (raw) {
    try {
      payload = JSON.parse(raw);
    } catch {
      payload = { value: raw };
    }
  }
  return { response, payload };
}

class WebDriver {
  constructor() {
    this.sessionId = null;
  }

  async request(method, endpoint, body, { optional = false } = {}) {
    const { response, payload } = await rawRequest(method, endpoint, body);
    const value = payload?.value;
    const error = value && typeof value === "object" ? value.error : undefined;
    if (!response.ok || error) {
      if (optional && (error === "no such element" || response.status === 404)) return null;
      const detail =
        value && typeof value === "object"
          ? value.message || JSON.stringify(value)
          : JSON.stringify(payload);
      fail(`${method} ${endpoint}: ${response.status} ${detail}`);
    }
    return value ?? payload;
  }

  async createSession() {
    const value = await this.request("POST", "/session", {
      capabilities: {
        alwaysMatch: {
          browserName: "wry",
          "tauri:options": { application: APP_BINARY },
        },
      },
    });
    this.sessionId = value?.sessionId || value?.["sessionId"];
    if (!this.sessionId) fail(`WebDriver session id missing: ${JSON.stringify(value)}`);
    await this.request("POST", this.endpoint("/timeouts"), { script: 60000, pageLoad: 30000, implicit: 0 });
    await this.waitFor(async () => Boolean(await this.find("css selector", "body", true)), 30000, "application body");
  }

  async quit() {
    if (!this.sessionId) return;
    const sessionId = this.sessionId;
    this.sessionId = null;
    try {
      await this.request("DELETE", `/session/${sessionId}`);
    } catch (error) {
      console.error(String(error));
    }
    await sleep(750);
  }

  endpoint(suffix) {
    if (!this.sessionId) fail("WebDriver session is not active");
    return `/session/${this.sessionId}${suffix}`;
  }

  elementId(value) {
    return value?.[ELEMENT_KEY] || value?.ELEMENT || null;
  }

  async find(using, value, optional = false) {
    const result = await this.request(
      "POST",
      this.endpoint("/element"),
      { using, value },
      { optional },
    );
    if (result === null) return null;
    const id = this.elementId(result);
    if (!id && !optional) fail(`element id missing for ${using}=${value}`);
    return id;
  }

  async findAll(using, value) {
    const result = await this.request("POST", this.endpoint("/elements"), { using, value });
    return Array.isArray(result) ? result.map((item) => this.elementId(item)).filter(Boolean) : [];
  }

  async clickElement(id) {
    await this.request("POST", this.endpoint(`/element/${id}/click`), {});
  }

  async clearElement(id) {
    await this.request("POST", this.endpoint(`/element/${id}/clear`), {});
  }

  async sendKeys(id, text) {
    await this.request("POST", this.endpoint(`/element/${id}/value`), {
      text,
      value: [...text],
    });
  }

  async text(id) {
    return await this.request("GET", this.endpoint(`/element/${id}/text`));
  }

  async attribute(id, name) {
    return await this.request("GET", this.endpoint(`/element/${id}/attribute/${encodeURIComponent(name)}`));
  }

  async executeSync(script, args = []) {
    return await this.request("POST", this.endpoint("/execute/sync"), { script, args });
  }

  async executeAsync(script, args = []) {
    return await this.request("POST", this.endpoint("/execute/async"), { script, args });
  }

  async invoke(command, args) {
    const result = await this.executeAsync(
      `const command = arguments[0];
       const payload = arguments[1];
       const done = arguments[arguments.length - 1];
       const invoke = window.__TAURI_INTERNALS__ && window.__TAURI_INTERNALS__.invoke;
       if (typeof invoke !== "function") {
         done({ ok: false, error: "Tauri invoke bridge unavailable" });
         return;
       }
       Promise.resolve(invoke(command, payload))
         .then((value) => done({ ok: true, value }))
         .catch((error) => {
           let detail;
           try { detail = typeof error === "string" ? error : JSON.stringify(error); }
           catch { detail = String(error); }
           done({ ok: false, error: detail });
         });`,
      [command, args],
    );
    if (!result?.ok) fail(`Tauri command ${command} failed: ${result?.error || "unknown error"}`);
    return result.value;
  }

  async invokeExpectFailure(command, args, expectedPattern) {
    const result = await this.executeAsync(
      `const command = arguments[0];
       const payload = arguments[1];
       const done = arguments[arguments.length - 1];
       const invoke = window.__TAURI_INTERNALS__ && window.__TAURI_INTERNALS__.invoke;
       if (typeof invoke !== "function") {
         done({ ok: false, error: "Tauri invoke bridge unavailable" });
         return;
       }
       Promise.resolve(invoke(command, payload))
         .then((value) => done({ ok: true, value }))
         .catch((error) => {
           let detail;
           try { detail = typeof error === "string" ? error : JSON.stringify(error); }
           catch { detail = String(error); }
           done({ ok: false, error: detail });
         });`,
      [command, args],
    );
    if (result?.ok) fail(`Tauri command ${command} unexpectedly succeeded`);
    const detail = String(result?.error || "");
    if (expectedPattern && !expectedPattern.test(detail)) {
      fail(`Tauri command ${command} failed for the wrong reason: ${detail}`);
    }
    return detail;
  }

  async doubleClickAria(label) {
    const clicked = await this.executeSync(
      `const label = arguments[0];
       const el = [...document.querySelectorAll('[aria-label]')].find(
         (node) => node.getAttribute('aria-label') === label && node.getClientRects().length
       );
       if (!el) return false;
       el.dispatchEvent(new MouseEvent('dblclick', { bubbles: true, cancelable: true, view: window }));
       return true;`,
      [label],
    );
    if (!clicked) fail(`could not double-click aria-label ${label}`);
  }

  async scrollListboxToBottom() {
    return await this.executeSync(
      `const boxes = [...document.querySelectorAll('[role="listbox"]')].filter(
         (node) => node.getClientRects().length && node.scrollHeight > node.clientHeight
       );
       const el = boxes[boxes.length - 1];
       if (!el) return false;
       el.scrollTop = el.scrollHeight;
       el.dispatchEvent(new Event('scroll', { bubbles: false }));
       return true;`,
    );
  }

  async waitFor(fn, timeoutMs, label) {
    const deadline = Date.now() + timeoutMs;
    let lastError;
    while (Date.now() < deadline) {
      try {
        const value = await fn();
        if (value) return value;
      } catch (error) {
        lastError = error;
      }
      await sleep(200);
    }
    if (lastError) throw lastError;
    fail(`timed out waiting for ${label}`);
  }

  async waitElement(using, value, timeoutMs = 15000) {
    return await this.waitFor(
      async () => await this.find(using, value, true),
      timeoutMs,
      `${using}=${value}`,
    );
  }

  async waitAbsent(using, value, timeoutMs = 10000) {
    await this.waitFor(
      async () => (await this.find(using, value, true)) === null,
      timeoutMs,
      `absence of ${using}=${value}`,
    );
  }

  async clickCss(selector) {
    const id = await this.waitElement("css selector", selector);
    await this.clickElement(id);
  }

  async clickText(text, timeoutMs = 15000) {
    const id = await this.waitElement("xpath", clickableTextXpath(text), timeoutMs);
    await this.clickElement(id);
  }

  async clickAnyText(texts, timeoutMs = 15000) {
    const deadline = Date.now() + timeoutMs;
    while (Date.now() < deadline) {
      for (const text of texts) {
        const id = await this.find("xpath", clickableTextXpath(text), true);
        if (id) {
          await this.clickElement(id);
          return text;
        }
      }
      await sleep(200);
    }
    fail(`timed out waiting for one of: ${texts.join(", ")}`);
  }

  async bodySnapshot(maxChars = 4096) {
    const body = await this.find("css selector", "body", true);
    if (!body) return "<body unavailable>";
    const value = await this.text(body);
    return String(value || "").replace(/\\s+/g, " ").trim().slice(0, maxChars);
  }

  async waitText(text, timeoutMs = 15000) {
    try {
      return await this.waitElement("xpath", textXpath(text), timeoutMs);
    } catch (error) {
      const snapshot = await this.bodySnapshot().catch(() => "<snapshot unavailable>");
      throw new Error(`${error.message}\nDOM text snapshot: ${snapshot}`);
    }
  }

  async waitOperationalStorage(timeoutMs = 20000) {
    try {
      await this.waitFor(
        async () => {
          const snapshot = await this.bodySnapshot(8192);
          if (snapshot.includes("Storage and MCP access are disabled")) {
            throw new Error(`app entered restricted recovery mode: ${snapshot}`);
          }
          return Boolean(
            await this.find(
              "css selector",
              '[role="button"][aria-label="Pilot Home"]',
              true,
            ),
          );
        },
        timeoutMs,
        "operational storage browser",
      );
    } catch (error) {
      const snapshot = await this.bodySnapshot(8192).catch(() => "<snapshot unavailable>");
      throw new Error(`${error.message}\nDOM text snapshot: ${snapshot || "<empty body>"}`);
    }
  }

  async setInput(selector, value) {
    const id = await this.waitElement("css selector", selector);
    await this.clearElement(id);
    await this.sendKeys(id, value);
  }

  async assertSelectPreselected(triggerSelector, label) {
    await this.clickCss(triggerSelector);
    const option = await this.waitElement(
      "xpath",
      `//*[@role='option' and contains(normalize-space(.), ${xpathLiteral(label)})]`,
    );
    const ariaSelected = await this.attribute(option, "aria-selected");
    const dataState = await this.attribute(option, "data-state");
    if (ariaSelected !== "true" && dataState !== "checked") {
      fail(
        `select was not preselected to ${label}; aria-selected=${ariaSelected} data-state=${dataState}`,
      );
    }
    await this.clickElement(option);
  }

  async selectOption(triggerSelector, label) {
    await this.clickCss(triggerSelector);
    const option = await this.waitElement(
      "xpath",
      `//*[@role='option' and contains(normalize-space(.), ${xpathLiteral(label)})]`,
    );
    await this.clickElement(option);
  }
}

function runState(command) {
  execFileSync(process.execPath, [STATE_SCRIPT, command, "--home", HOME_DIR], {
    cwd: ROOT_DIR,
    env: { ...process.env, HOME: HOME_DIR },
    stdio: "inherit",
  });
}

async function waitForDriver(driverProcess) {
  const deadline = Date.now() + 15000;
  while (Date.now() < deadline) {
    if (driverProcess.exitCode !== null) {
      fail(`tauri-driver exited early with status ${driverProcess.exitCode}`);
    }
    try {
      const { response } = await rawRequest("GET", "/status");
      if (response.ok) return;
    } catch {
      // keep polling
    }
    await sleep(200);
  }
  fail("tauri-driver did not become ready on port 4444");
}

function assertGeneralServeFailsClosed() {
  const result = spawnSync(SIDECAR, ["serve", "--transport", "stdio"], {
    cwd: ROOT_DIR,
    env: { ...process.env, HOME: HOME_DIR },
    encoding: "utf8",
    input: "",
    timeout: 5000,
  });
  if (result.error && result.error.code === "ETIMEDOUT") {
    fail("ordinary stdio serve stayed alive while the persisted Agent Access gate was disabled");
  }
  if (result.status === 0) {
    fail("ordinary stdio serve succeeded while the persisted Agent Access gate was disabled");
  }
  console.log("ordinary_stdio_disabled_gate=passed");
}

async function openStorageMenu(driver) {
  await driver.clickCss('button[aria-label="Storage actions"]');
}

async function storageOnlySession(driver) {
  console.log("phase=storage-only-onboarding");
  await driver.createSession();
  try {
    await driver.waitOperationalStorage(20000);
    await driver.waitElement("css selector", '[aria-label="pilot-browser.txt"]', 20000);
    await openStorageMenu(driver);
    await driver.clickText("Setup Guide");
    await driver.waitText("Welcome to Infimount");
    await driver.clickText("Browse storage only");
    await driver.waitAbsent("xpath", textXpath("Welcome to Infimount"));
    await driver.waitText("pilot-browser.txt");
    await driver.waitElement("css selector", 'button[aria-label^="Agent Access:"]');
  } finally {
    await driver.quit();
  }
  runState("assert-storage-only");
}

async function createWorkspace(driver, { name, allowWrites }) {
  await openStorageMenu(driver);
  await driver.clickText("Agent Workspaces");
  await driver.waitText("Create workspace");
  await driver.setInput("#workspace-name", name);

  const writeSwitch = await driver.waitElement("css selector", 'button[aria-label="Allow agent writes"]');
  const state = await driver.attribute(writeSwitch, "data-state");
  if (allowWrites && state !== "checked") await driver.clickElement(writeSwitch);
  if (!allowWrites && state === "checked") await driver.clickElement(writeSwitch);

  await driver.clickText("Create workspace", 20000);
  await driver.waitText(name, 20000);
  await driver.waitElement("xpath", clickableTextXpath("Connect agent"), 20000);
}

async function connectPreparedWorkspace(driver, expectedName) {
  await driver.clickText("Connect agent");
  await driver.waitText("Connect an AI client to one scoped workspace");

  await driver.assertSelectPreselected(
    '[aria-label="Agent Access workspace"]',
    expectedName,
  );

  await driver.clickAnyText(["Prepare agent access", "Re-check agent access"], 30000);
  await driver.waitElement("xpath", clickableTextXpath("Re-check agent access"), 30000);
  await driver.waitText("stdio is on demand", 30000);
  const httpStartButtons = await driver.findAll(
    "xpath",
    "//button[contains(normalize-space(.), 'Start HTTP server')]",
  );
  if (httpStartButtons.length !== 0) {
    fail("stdio Agent Access unexpectedly exposed a Start HTTP server action");
  }
}

async function readOnlyAgentAccessSession(driver) {
  console.log("phase=legacy-root-read-only-agent-access");
  await driver.createSession();
  try {
    await driver.waitAbsent("xpath", textXpath("Welcome to Infimount"));
    await driver.waitOperationalStorage(20000);
    await driver.waitElement("css selector", '[aria-label="pilot-browser.txt"]', 20000);
    await driver.waitElement("css selector", 'button[aria-label^="Agent Access:"]');

    await createWorkspace(driver, { name: READ_ONLY_WORKSPACE, allowWrites: false });

    const workspaceDir = path.join(HOME_DIR, READ_ONLY_ROOT);
    fs.mkdirSync(workspaceDir, { recursive: true });
    fs.writeFileSync(path.join(workspaceDir, "sample.txt"), "inside release pilot fixture\n");
    fs.mkdirSync(path.join(HOME_DIR, "outside"), { recursive: true });
    fs.writeFileSync(path.join(HOME_DIR, "outside", "denied.txt"), "outside release pilot fixture\n");

    await connectPreparedWorkspace(driver, READ_ONLY_WORKSPACE);
    await driver.clickText("Run safety probe", 30000);
    await driver.waitText("Safety probe passed.", 45000);
  } finally {
    await driver.quit();
  }
  runState("assert-read-only-agent-access");
}

async function readWriteAndHttpSession(driver) {
  console.log("phase=read-write-and-http-lifecycle");
  await driver.createSession();
  try {
    await createWorkspace(driver, { name: WRITABLE_WORKSPACE, allowWrites: true });
    await connectPreparedWorkspace(driver, WRITABLE_WORKSPACE);
    runState("assert-read-write-agent-access");

    await driver.clickText("Advanced MCP settings");
    await driver.waitText("Advanced MCP Settings");

    await driver.selectOption('[aria-label="MCP transport"]', "http");

    await driver.clickText("Save & Start HTTP Server", 30000);
    await driver.waitText("HTTP server is live.", 30000);
    await driver.waitElement("xpath", clickableTextXpath("Stop HTTP Server"));

    await driver.clickText("Stop HTTP Server", 30000);
    await driver.waitText("HTTP server is not running.", 30000);
  } finally {
    await driver.quit();
  }

  runState("assert-http-stopped");
  assertGeneralServeFailsClosed();
}

async function guidedReenableSession(driver) {
  console.log("phase=guided-reenable");
  await driver.createSession();
  try {
    await driver.clickCss('button[aria-label^="Agent Access:"]');
    await driver.waitText("Connect an AI client to one scoped workspace");

    await driver.selectOption('[aria-label="Agent Access workspace"]', READ_ONLY_WORKSPACE);
    await driver.clickAnyText(["Prepare agent access", "Re-check agent access"], 30000);
    await driver.waitElement("xpath", clickableTextXpath("Re-check agent access"), 30000);
    await driver.waitText("stdio is on demand", 30000);
  } finally {
    await driver.quit();
  }

  runState("assert-guided-reenable");
}


function runKitCheck(kind, outputsDir) {
  execFileSync(process.execPath, [
    PILOT_KIT_CORE,
    "check",
    kind,
    "--kit",
    PILOT_KIT,
    "--outputs",
    outputsDir,
  ], { cwd: ROOT_DIR, stdio: "inherit" });
}

function writeTaskOutputs(kind, outputsDir) {
  fs.mkdirSync(outputsDir, { recursive: true });
  if (kind === "coding") {
    fs.writeFileSync(
      path.join(outputsDir, "review.md"),
      "Root cause: the implementation deduplicates by customerId, so distinct invoices for one customer are dropped. The fix deduplicates by invoice id while retaining closed-invoice filtering.\n",
    );
    fs.writeFileSync(
      path.join(outputsDir, "patch.diff"),
      `diff --git a/src/invoiceTotals.mjs b/src/invoiceTotals.mjs
--- a/src/invoiceTotals.mjs
+++ b/src/invoiceTotals.mjs
@@ -1,11 +1,11 @@
 export function totalOpenInvoices(invoices) {
-  const seenCustomers = new Set();
+  const seenInvoices = new Set();
   let total = 0;
 
   for (const invoice of invoices) {
     if (invoice.status !== "open") continue;
-    if (seenCustomers.has(invoice.customerId)) continue;
-    seenCustomers.add(invoice.customerId);
+    if (seenInvoices.has(invoice.id)) continue;
+    seenInvoices.add(invoice.id);
     total += invoice.amount;
   }
 
`,
    );
    return;
  }
  if (kind === "document") {
    fs.writeFileSync(
      path.join(outputsDir, "summary.md"),
      "The pilot remains targeted for 14 October, coordinated by Priya, for 12 customer accounts. Security approval of the data-retention wording remains the unresolved launch dependency. Product approved in-app copy on 3 October. Seven pilot customers requested a concise setup guide. SSO expansion is explicitly out of scope for this pilot.\n",
    );
    fs.writeFileSync(
      path.join(outputsDir, "action-items.md"),
      "- Mateo: finish the rollback checklist by 10 October.\n- Asha: publish the two-page setup guide by 11 October.\n- Security: resolve the data-retention wording; no approval date is currently committed.\n",
    );
    return;
  }
  if (kind === "data-analysis") {
    const rows = fs.readFileSync(path.join(PILOT_KIT, "data", "source", "orders.csv"), "utf8")
      .trim().split(/\\r?\\n/).slice(1).map((line) => line.split(","));
    let gross = 0;
    let refundAmount = 0;
    let refundedOrders = 0;
    const regionNet = new Map();
    const productNet = new Map();
    for (const [, region, , product, quantityRaw, priceRaw, refundedRaw] of rows) {
      const value = Number(quantityRaw) * Number(priceRaw);
      const refunded = Number(refundedRaw) === 1;
      gross += value;
      if (refunded) {
        refundAmount += value;
        refundedOrders += 1;
      }
      const net = refunded ? 0 : value;
      regionNet.set(region, (regionNet.get(region) || 0) + net);
      productNet.set(product, (productNet.get(product) || 0) + net);
    }
    const topKey = (map) => [...map.entries()].sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0]))[0][0];
    const metrics = {
      gross_revenue: String(gross),
      net_revenue: String(gross - refundAmount),
      refund_amount: String(refundAmount),
      refunded_orders: String(refundedOrders),
      order_count: String(rows.length),
      refund_rate_pct: ((refundedOrders / rows.length) * 100).toFixed(2),
      top_region_by_net_revenue: topKey(regionNet),
      top_product_by_net_revenue: topKey(productNet),
    };
    fs.writeFileSync(path.join(outputsDir, "findings.md"), "Refunded orders are treated as zero net revenue. Metrics were computed from the prepared CSV snapshot.\n");
    fs.writeFileSync(
      path.join(outputsDir, "summary.csv"),
      `metric,value\\n${Object.entries(metrics).map(([key, value]) => `${key},${value}`).join("\\n")}\\n`,
    );
    return;
  }
  fail(`unknown Agent Task workload: ${kind}`);
}

async function paginationSession(driver) {
  console.log("phase=pagination-auto-continuation");
  await driver.createSession();
  try {
    await driver.waitOperationalStorage(20000);
    await driver.waitElement("xpath", `//*[@role='option' and @aria-label='pagination']`, 20000);
    await driver.doubleClickAria("pagination");
    await driver.waitElement("xpath", `//*[@role='option' and @aria-label='file-000.txt']`, 20000);

    const loadMore = await driver.find("xpath", "//button[normalize-space(.)='Load more']", true);
    if (loadMore) {
      const className = String(await driver.attribute(loadMore, "class") || "");
      assert(className.includes("sr-only"), "pagination exposed a visible Load more control");
    }

    let lastVisible = false;
    for (let attempt = 0; attempt < 12 && !lastVisible; attempt += 1) {
      await driver.scrollListboxToBottom();
      await sleep(300);
      lastVisible = Boolean(await driver.find("xpath", `//*[@role='option' and @aria-label='file-449.txt']`, true));
    }
    assert(lastVisible, "automatic pagination did not reach file-449.txt");
    console.log("pagination_450_entries=passed");
    console.log("visible_load_more=no");
  } finally {
    await driver.quit();
  }

  console.log("phase=pagination-stale-cursor-recovery");
  await driver.createSession();
  try {
    await driver.waitElement("xpath", `//*[@role='option' and @aria-label='pagination-stale']`, 20000);
    await driver.doubleClickAria("pagination-stale");
    await driver.waitElement("xpath", `//*[@role='option' and @aria-label='file-000.txt']`, 20000);

    await driver.invoke("create_workspace_atomic", {
      request: {
        storageId: STORAGE_ID,
        name: "Pagination cursor bump",
        rootPath: "/agent-workspaces/pagination-cursor-bump",
        templateId: "custom",
        accessProfile: "read_only",
        applyPolicy: true,
      },
    });

    let recovered = false;
    for (let attempt = 0; attempt < 16 && !recovered; attempt += 1) {
      await driver.scrollListboxToBottom();
      await sleep(350);
      recovered = Boolean(await driver.find("xpath", `//*[@role='option' and @aria-label='file-449.txt']`, true));
    }
    assert(recovered, "stale-cursor recovery did not resume automatic pagination");
    const errorText = await driver.find("xpath", textXpath("More files could not be loaded."), true);
    assert(!errorText, "stale cursor surfaced as a user-visible pagination failure");
    console.log("pagination_stale_cursor_recovery=passed");
  } finally {
    await driver.quit();
  }
}

async function runAgentTaskWorkload(driver, kind, request, publicationDir) {
  const sourceRoot = path.join(HOME_DIR, ...request.sourcePaths[0].split("/"));
  const beforeDigest = treeDigest(sourceRoot);

  const preflight = await driver.invoke("preflight_agent_task", { request });
  assert(preflight.fileCount > 0, `${kind} preflight found no files`);
  const prepared = await driver.invoke("prepare_agent_task", { request });
  assert(prepared.preparedFiles === preflight.fileCount, `${kind} prepared file count mismatch`);

  const outputsDir = path.join(HOME_DIR, WRITABLE_ROOT, prepared.taskRoot, "outputs");
  writeTaskOutputs(kind, outputsDir);
  runKitCheck(kind, outputsDir);

  let review = await driver.invoke("review_agent_task_outputs", {
    request: { workspaceId: request.workspaceId, taskId: prepared.taskId },
  });
  assert(review.fileCount >= 2, `${kind} output review did not enumerate expected files`);

  const toPublication = (conflictPolicy) => ({
    workspaceId: request.workspaceId,
    taskId: prepared.taskId,
    outputs: review.files.map((file) => ({
      taskPath: file.taskPath,
      byteSize: file.byteSize,
      sha256: file.sha256,
    })),
    destinationStorageId: STORAGE_ID,
    destinationDir: publicationDir,
    conflictPolicy,
  });

  let publication;
  if (kind === "coding") {
    const firstRequest = toPublication("fail");
    const firstPreview = await driver.invoke("preview_agent_task_publication", { request: firstRequest });
    assert(firstPreview.canPublish === true, "coding publication preview should initially be publishable");
    fs.appendFileSync(path.join(outputsDir, "review.md"), "Stale-preview mutation probe.\n");
    await driver.invokeExpectFailure(
      "publish_agent_task_outputs",
      { request: { publication: firstRequest, previewToken: firstPreview.previewToken } },
      /(changed since review|changed|hash|review)/i,
    );
    review = await driver.invoke("review_agent_task_outputs", {
      request: { workspaceId: request.workspaceId, taskId: prepared.taskId },
    });
    runKitCheck(kind, outputsDir);
    const freshRequest = toPublication("fail");
    const freshPreview = await driver.invoke("preview_agent_task_publication", { request: freshRequest });
    publication = await driver.invoke("publish_agent_task_outputs", {
      request: { publication: freshRequest, previewToken: freshPreview.previewToken },
    });
    console.log("agent_task_stale_preview_rejected=passed");
  } else if (kind === "document") {
    const destination = path.join(HOME_DIR, publicationDir);
    fs.mkdirSync(destination, { recursive: true });
    const sentinel = path.join(destination, "summary.md");
    fs.writeFileSync(sentinel, "Existing destination sentinel. This file must never be overwritten by the pilot.\n");
    const sentinelSha = sha256File(sentinel);

    const failRequest = toPublication("fail");
    const conflictPreview = await driver.invoke("preview_agent_task_publication", { request: failRequest });
    assert(conflictPreview.canPublish === false && conflictPreview.conflictCount >= 1, "fail-on-conflict preview did not block publication");
    await driver.invokeExpectFailure(
      "publish_agent_task_outputs",
      { request: { publication: failRequest, previewToken: conflictPreview.previewToken } },
      /conflict/i,
    );
    await driver.invokeExpectFailure(
      "preview_agent_task_publication",
      { request: { ...failRequest, conflictPolicy: "overwrite" } },
      /(overwrite|variant|conflict)/i,
    );

    const renameRequest = toPublication("rename");
    const renamePreview = await driver.invoke("preview_agent_task_publication", { request: renameRequest });
    assert(renamePreview.canPublish === true && renamePreview.renameCount >= 1, "rename preview did not resolve conflict");
    publication = await driver.invoke("publish_agent_task_outputs", {
      request: { publication: renameRequest, previewToken: renamePreview.previewToken },
    });
    assert(sha256File(sentinel) === sentinelSha, "document conflict publication overwrote the sentinel");
    console.log("agent_task_fail_conflict_rejected=passed");
    console.log("agent_task_overwrite_unavailable=passed");
  } else {
    const publishRequest = toPublication("fail");
    const preview = await driver.invoke("preview_agent_task_publication", { request: publishRequest });
    assert(preview.canPublish === true, "data publication preview is not publishable");
    publication = await driver.invoke("publish_agent_task_outputs", {
      request: { publication: publishRequest, previewToken: preview.previewToken },
    });
  }

  assert(publication.files.length === review.files.length, `${kind} publication file count mismatch`);
  for (const file of publication.files) {
    const source = path.join(HOME_DIR, WRITABLE_ROOT, prepared.taskRoot, ...file.taskPath.split("/"));
    const destination = path.join(HOME_DIR, ...file.destinationPath.split("/"));
    assert(fs.existsSync(destination), `${kind} destination file missing`);
    assert(sha256File(source) === sha256File(destination), `${kind} destination bytes do not match reviewed output`);
  }
  const receipt = path.join(HOME_DIR, WRITABLE_ROOT, ...publication.receiptPath.split("/"));
  assert(fs.existsSync(receipt), `${kind} publication receipt missing`);

  const afterDigest = treeDigest(sourceRoot);
  assert(JSON.stringify(beforeDigest) === JSON.stringify(afterDigest), `${kind} source bytes changed during Agent Task flow`);
  console.log(`agent_task_${kind}=passed`);
  return { taskId: prepared.taskId, receiptPath: publication.receiptPath, receiptSha256: sha256File(receipt) };
}

async function agentTasksSession(driver) {
  console.log("phase=agent-task-workloads-publication");
  await driver.createSession();
  try {
    const workspace = workspaceRecord(WRITABLE_WORKSPACE);
    const workloads = [
      {
        kind: "coding",
        sourcePaths: ["pilot-kit/coding/source"],
        title: "Coding pilot",
        objective: "Diagnose the invoice total defect and provide review.md plus patch.diff.",
        requestedOutputs: ["outputs/review.md", "outputs/patch.diff"],
        publicationDir: "pilot-published/coding",
      },
      {
        kind: "document",
        sourcePaths: ["pilot-kit/document/source"],
        title: "Document pilot",
        objective: "Synthesize the launch notes without inventing decisions.",
        requestedOutputs: ["outputs/summary.md", "outputs/action-items.md"],
        publicationDir: "pilot-published/document",
      },
      {
        kind: "data-analysis",
        sourcePaths: ["pilot-kit/data/source"],
        title: "Data analysis pilot",
        objective: "Analyze orders.csv and produce findings.md plus summary.csv.",
        requestedOutputs: ["outputs/findings.md", "outputs/summary.csv"],
        publicationDir: "pilot-published/data",
      },
    ];

    const receipts = [];
    for (const workload of workloads) {
      receipts.push(await runAgentTaskWorkload(
        driver,
        workload.kind,
        {
          sourceStorageId: STORAGE_ID,
          sourcePaths: workload.sourcePaths,
          workspaceId: workspace.id,
          title: workload.title,
          objective: workload.objective,
          requestedOutputs: workload.requestedOutputs,
        },
        workload.publicationDir,
      ));
    }

    assert(new Set(receipts.map((item) => item.receiptPath)).size === receipts.length, "Agent Task publication receipts are not unique");
    console.log("agent_task_unique_receipts=passed");
  } finally {
    await driver.quit();
  }
}

async function main() {
  if (!HOME_DIR || HOME_DIR === path.parse(HOME_DIR).root) fail("unsafe pilot HOME");
  requireFile(APP_BINARY, "desktop application");
  requireFile(SIDECAR, "MCP sidecar");
  requireFile(STATE_SCRIPT, "release pilot state script");
  requireFile(PILOT_KIT_CORE, "Agent Task pilot kit core");
  requireDirectory(PILOT_KIT, "Agent Task pilot kit");

  const driver = new WebDriver();
  let driverStdout = "";
  let driverStderr = "";
  const driverProcess = spawn("tauri-driver", [], {
    cwd: ROOT_DIR,
    env: { ...process.env, HOME: HOME_DIR },
    stdio: ["ignore", "pipe", "pipe"],
  });
  driverProcess.stdout.on("data", (chunk) => {
    driverStdout = (driverStdout + chunk.toString()).slice(-20000);
  });
  driverProcess.stderr.on("data", (chunk) => {
    driverStderr = (driverStderr + chunk.toString()).slice(-20000);
  });

  try {
    await waitForDriver(driverProcess);
    await storageOnlySession(driver);
    await readOnlyAgentAccessSession(driver);
    await readWriteAndHttpSession(driver);
    await guidedReenableSession(driver);
    await paginationSession(driver);
    await agentTasksSession(driver);
    runState("summary");
    console.log("pagination_extended=passed");
    console.log("agent_tasks_extended=passed");
    console.log("release_pilot_webdriver=passed");
  } catch (error) {
    console.error(String(error?.stack || error));
    if (driverStdout.trim()) console.error(`tauri-driver stdout:\n${driverStdout}`);
    if (driverStderr.trim()) console.error(`tauri-driver stderr:\n${driverStderr}`);
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
