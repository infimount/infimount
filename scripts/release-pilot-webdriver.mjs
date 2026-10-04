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
const READ_ONLY_WORKSPACE = "Release pilot workspace";
const READ_ONLY_ROOT = "agent-workspaces/release-pilot-workspace";
const WRITABLE_WORKSPACE = "Release pilot writable";
const AGENT_TASK_SOURCE_FILE = "agent-task-source.txt";
const AGENT_TASK_SOURCE_CONTENT = "release pilot source bytes must remain unchanged\n";
const AGENT_TASK_OUTPUT_V1 = "deterministic agent output v1\n";
const AGENT_TASK_OUTPUT_V2 = "deterministic agent output v2\n";
const AGENT_TASK_DESTINATION_DIR = "publish-destination";

function fail(message) {
  throw new Error(`Release pilot WebDriver failed: ${message}`);
}

function requireFile(file, label) {
  if (!file || !fs.existsSync(file) || !fs.statSync(file).isFile()) {
    fail(`${label} is missing: ${file}`);
  }
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

  async displayed(id) {
    return await this.request("GET", this.endpoint(`/element/${id}/displayed`));
  }

  async execute(script, args = []) {
    return await this.request("POST", this.endpoint("/execute/sync"), { script, args });
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

  async clickLocated(using, value, timeoutMs = 15000) {
    await this.waitFor(
      async () => {
        const id = await this.find(using, value, true);
        if (!id) return false;
        try {
          await this.execute(
            `const el = arguments[0];
             if (el && typeof el.scrollIntoView === "function") {
               el.scrollIntoView({ block: "center", inline: "nearest" });
             }
             return true;`,
            [{ [ELEMENT_KEY]: id }],
          );
          await sleep(50);
          await this.clickElement(id);
          return true;
        } catch (error) {
          const message = String(error?.message || error);
          if (/element not interactable|stale element|click intercepted|no such element/i.test(message)) {
            return false;
          }
          throw error;
        }
      },
      timeoutMs,
      `clickable ${using}=${value}`,
    );
  }

  async clickCss(selector, timeoutMs = 15000) {
    await this.clickLocated("css selector", selector, timeoutMs);
  }

  async clickText(text, timeoutMs = 15000) {
    await this.clickLocated("xpath", clickableTextXpath(text), timeoutMs);
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
    let id = await this.waitElement("css selector", selector);
    await this.clearElement(id);
    // Controlled React inputs may be replaced after clear. Reacquire the
    // element before every subsequent WebDriver action instead of retaining a
    // stale node identifier.
    id = await this.waitElement("css selector", selector);
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

  async assertNoSelectOption(triggerSelector, forbiddenText, closeWithLabel) {
    await this.clickCss(triggerSelector);
    const forbidden = await this.find(
      "xpath",
      `//*[@role='option' and contains(translate(normalize-space(.), 'ABCDEFGHIJKLMNOPQRSTUVWXYZ', 'abcdefghijklmnopqrstuvwxyz'), ${xpathLiteral(forbiddenText.toLowerCase())})]`,
      true,
    );
    if (forbidden) fail(`select unexpectedly exposes forbidden option: ${forbiddenText}`);
    const close = await this.waitElement(
      "xpath",
      `//*[@role='option' and contains(normalize-space(.), ${xpathLiteral(closeWithLabel)})]`,
    );
    await this.clickElement(close);
  }

  async closeDialogContaining(text) {
    const dialog = await this.waitElement(
      "xpath",
      `//*[@role='dialog' and contains(normalize-space(.), ${xpathLiteral(text)})]`,
    );
    const close = await this.find(
      "xpath",
      `//*[@role='dialog' and contains(normalize-space(.), ${xpathLiteral(text)})]//button[.//span[normalize-space(.)='Close']]`,
      true,
    );
    if (!close) fail(`close control missing for dialog containing ${text}`);
    await this.clickElement(close);
    await this.waitFor(
      async () => !(await this.displayed(dialog).catch(() => false)),
      10000,
      `dialog containing ${text} to close`,
    );
  }

  async scrollFileListToEnd() {
    const listbox = await this.waitElement("css selector", '[role="listbox"]');
    await this.execute(
      `const el = arguments[0];
       el.scrollTop = 0;
       el.dispatchEvent(new Event("scroll", { bubbles: false }));
       el.scrollTop = el.scrollHeight;
       el.dispatchEvent(new Event("scroll", { bubbles: false }));
       return { scrollTop: el.scrollTop, scrollHeight: el.scrollHeight, clientHeight: el.clientHeight };`,
      [{ [ELEMENT_KEY]: listbox }],
    );
  }
}

function sha256Bytes(bytes) {
  return crypto.createHash("sha256").update(bytes).digest("hex");
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

async function assertTaskScopedSidecarWorks(taskId) {
  const mcpSettings = JSON.parse(
    fs.readFileSync(path.join(HOME_DIR, ".infimount", "mcp_settings.json"), "utf8"),
  );
  if (mcpSettings.enabled !== false) {
    fail("task-scoped sidecar independence probe requires general Agent Access to be disabled");
  }

  const storages = JSON.parse(
    fs.readFileSync(path.join(HOME_DIR, ".infimount", "storages.json"), "utf8"),
  );
  const workspaceDoc = JSON.parse(
    fs.readFileSync(path.join(HOME_DIR, ".infimount", "workspaces.json"), "utf8"),
  );
  const workspace = (workspaceDoc.workspaces || []).find(
    (item) => item.name === WRITABLE_WORKSPACE,
  );
  if (!workspace) fail("read-write workspace missing for task-scoped sidecar probe");
  const storage = storages.find((item) => item.id === workspace.storageId);
  if (!storage) fail("workspace storage missing for task-scoped sidecar probe");
  if (!workspace.policyRuleId || !workspace.storageNamespaceFingerprint) {
    fail("workspace binding metadata missing for task-scoped sidecar probe");
  }

  const workspacePrefix = workspace.rootPath;
  const taskPrefix = `${String(workspace.rootPath).replace(/^\/+|\/+$/g, "")}/tasks/${taskId}`;
  const outputsPrefix = `${taskPrefix}/outputs`;
  const taskInputPath = `/${storage.name}/${taskPrefix}/inputs/${AGENT_TASK_SOURCE_FILE}`;
  const outsidePath = `/${storage.name}/pilot-browser.txt`;

  const args = [
    "serve-agent-task",
    "--task-id", taskId,
    "--storage-id", storage.id,
    "--workspace-id", workspace.id,
    "--policy-rule-id", workspace.policyRuleId,
    "--storage-namespace-fingerprint", workspace.storageNamespaceFingerprint,
    "--workspace-prefix", workspacePrefix,
    "--task-prefix", taskPrefix,
    "--outputs-prefix", outputsPrefix,
  ];
  const child = spawn(SIDECAR, args, {
    cwd: ROOT_DIR,
    env: { ...process.env, HOME: HOME_DIR },
    stdio: ["pipe", "pipe", "pipe"],
  });

  const responses = new Map();
  let stdoutBuffer = "";
  let stderrTail = "";
  let protocolError = null;

  child.stdout.setEncoding("utf8");
  child.stderr.setEncoding("utf8");
  child.stderr.on("data", (chunk) => {
    stderrTail = (stderrTail + chunk).slice(-4000);
  });
  child.stdout.on("data", (chunk) => {
    stdoutBuffer += chunk;
    for (;;) {
      const newline = stdoutBuffer.indexOf("\n");
      if (newline < 0) break;
      const line = stdoutBuffer.slice(0, newline).trim();
      stdoutBuffer = stdoutBuffer.slice(newline + 1);
      if (!line) continue;
      try {
        const message = JSON.parse(line);
        if (message && Number.isInteger(message.id)) {
          responses.set(message.id, message);
        }
      } catch {
        protocolError = `task-scoped sidecar emitted non-JSON stdout: ${line.slice(0, 500)}`;
      }
    }
  });

  const send = (message) => {
    if (!child.stdin.writable) {
      fail("task-scoped packaged sidecar stdin closed unexpectedly");
    }
    child.stdin.write(`${JSON.stringify(message)}\n`);
  };

  const waitResponse = async (id, timeoutMs = 7000) => {
    const deadline = Date.now() + timeoutMs;
    while (Date.now() < deadline) {
      if (protocolError) fail(protocolError);
      if (responses.has(id)) return responses.get(id);
      if (child.exitCode !== null) {
        fail(
          `task-scoped packaged sidecar exited before response ${id} with ${child.exitCode}: ${stderrTail}`,
        );
      }
      await sleep(25);
    }
    fail(`timed out waiting for task-scoped MCP response ${id}: ${stderrTail}`);
  };

  try {
    send({
      jsonrpc: "2.0",
      id: 1,
      method: "initialize",
      params: {
        protocolVersion: "2025-06-18",
        capabilities: {},
        clientInfo: { name: "infimount-release-pilot", version: "1" },
      },
    });
    const initialize = await waitResponse(1);
    if (!initialize?.result || initialize.error) {
      fail(`task-scoped packaged sidecar MCP initialize failed: ${JSON.stringify(initialize)}`);
    }

    send({ jsonrpc: "2.0", method: "notifications/initialized" });
    send({ jsonrpc: "2.0", id: 2, method: "tools/list", params: {} });
    const toolsResponse = await waitResponse(2);
    const toolNames = (toolsResponse?.result?.tools || [])
      .map((tool) => tool.name)
      .filter(Boolean)
      .sort();
    const expectedTools = [
      "list_dir",
      "mkdir",
      "read_file",
      "search_paths",
      "stat_path",
      "write_file",
    ].sort();
    if (JSON.stringify(toolNames) !== JSON.stringify(expectedTools)) {
      fail(`task-scoped sidecar tool set mismatch: ${JSON.stringify(toolNames)}`);
    }

    send({
      jsonrpc: "2.0",
      id: 3,
      method: "tools/call",
      params: {
        name: "read_file",
        arguments: { path: taskInputPath, as_text: true },
      },
    });
    const allowed = await waitResponse(3);
    const allowedText = JSON.stringify(allowed);
    if (
      !allowed ||
      allowed.error ||
      allowed?.result?.isError === true ||
      !allowedText.includes(AGENT_TASK_SOURCE_CONTENT.trim())
    ) {
      fail(`task-scoped sidecar could not read prepared input: ${allowedText.slice(0, 1500)}`);
    }

    send({
      jsonrpc: "2.0",
      id: 4,
      method: "tools/call",
      params: {
        name: "read_file",
        arguments: { path: outsidePath, as_text: true },
      },
    });
    const denied = await waitResponse(4);
    const deniedText = JSON.stringify(denied);
    if (!denied || !deniedText.includes("ERR_SESSION_FORBIDDEN")) {
      fail(`task-scoped sidecar did not deny out-of-task read: ${deniedText.slice(0, 1500)}`);
    }

    console.log("agent_task_scoped_sidecar_independent_gate=passed");
    console.log("agent_task_scoped_sidecar_tool_surface=passed");
    console.log("agent_task_scoped_sidecar_confinement=passed");
  } finally {
    child.stdin.end();
    const deadline = Date.now() + 3000;
    while (child.exitCode === null && Date.now() < deadline) {
      await sleep(25);
    }
    if (child.exitCode === null) {
      child.kill("SIGKILL");
      await sleep(100);
    }
  }
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
  console.log("phase=legacy-root-read-only-agent-access-and-stale-pagination");
  await driver.createSession();
  try {
    await driver.waitAbsent("xpath", textXpath("Welcome to Infimount"));
    await driver.waitOperationalStorage(20000);
    await driver.waitElement("css selector", '[aria-label="pilot-browser.txt"]', 20000);
    await driver.waitElement("css selector", 'button[aria-label^="Agent Access:"]');

    // Load the first 200-entry page before any workspace mutation. Workspace
    // binding below increments the storage revision, intentionally making this
    // continuation cursor stale while the browser stays mounted.
    const paginationFolder = await driver.waitElement(
      "css selector",
      '[role="option"][aria-label="pagination"]',
      20000,
    );
    await driver.request(
      "POST",
      driver.endpoint(`/element/${paginationFolder}/value`),
      { text: "\\uE007", value: ["\\uE007"] },
    ).catch(() => null);
    await driver.execute(
      `const el = document.querySelector('[role="option"][aria-label="pagination"]');
       if (!el) return false;
       el.dispatchEvent(new MouseEvent("dblclick", { bubbles: true, cancelable: true, view: window }));
       return true;`,
    );
    await driver.waitElement("css selector", '[aria-label="page-0000.txt"]', 20000);
    const hiddenLoadMore = await driver.waitElement(
      "xpath",
      "//button[contains(normalize-space(.), 'Load more')]",
      20000,
    );
    const loadMoreClass = String(await driver.attribute(hiddenLoadMore, "class") || "");
    if (!loadMoreClass.split(/\s+/).includes("sr-only")) {
      fail(`pagination Load more fallback is not screen-reader-only: ${loadMoreClass}`);
    }
    console.log("pagination_first_page_loaded=yes");
    console.log("visible_load_more_control=no");

    await createWorkspace(driver, { name: READ_ONLY_WORKSPACE, allowWrites: false });

    const workspaceDir = path.join(HOME_DIR, READ_ONLY_ROOT);
    fs.mkdirSync(workspaceDir, { recursive: true });
    fs.writeFileSync(path.join(workspaceDir, "sample.txt"), "inside release pilot fixture\n");
    fs.mkdirSync(path.join(HOME_DIR, "outside"), { recursive: true });
    fs.writeFileSync(path.join(HOME_DIR, "outside", "denied.txt"), "outside release pilot fixture\n");

    await connectPreparedWorkspace(driver, READ_ONLY_WORKSPACE);
    await driver.clickText("Run safety probe", 30000);
    await driver.waitText("Safety probe passed.", 45000);

    // Return to the still-mounted /pagination browser. Its original cursor is
    // now storage-revision-stale. Scrolling must recover from page one
    // internally and then auto-continue to the last fixture.
    await driver.closeDialogContaining("Connect an AI client to one scoped workspace");
    let reachedLast = false;
    for (let attempt = 0; attempt < 8; attempt += 1) {
      await driver.scrollFileListToEnd();
      await sleep(800);
      const last = await driver.find("css selector", '[aria-label="page-0259.txt"]', true);
      if (last && await driver.displayed(last)) {
        reachedLast = true;
        break;
      }
      const snapshot = await driver.bodySnapshot(12000);
      if (snapshot.includes("More files could not be loaded")) {
        fail(`pagination surfaced a continuation error after stale cursor recovery: ${snapshot}`);
      }
    }
    if (!reachedLast) {
      fail("pagination did not auto-continue to page-0259.txt after stale cursor recovery");
    }
    const remainingLoadMore = await driver.find(
      "xpath",
      "//button[contains(normalize-space(.), 'Load more')]",
      true,
    );
    if (remainingLoadMore) {
      const remainingClass = String(await driver.attribute(remainingLoadMore, "class") || "");
      if (!remainingClass.split(/\s+/).includes("sr-only")) {
        fail(`pagination Load more fallback became visible after continuation: ${remainingClass}`);
      }
    }
    console.log("pagination_auto_continuation=passed");
    console.log("stale_cursor_recovery=passed");
  } finally {
    await driver.quit();
  }
  runState("assert-read-only-agent-access");
  runState("assert-pagination");
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

async function agentTaskPublicationSession(driver) {
  console.log("phase=agent-task-publication-safety");
  const sourcePath = path.join(HOME_DIR, AGENT_TASK_SOURCE_FILE);
  const sourceBefore = fs.readFileSync(sourcePath);
  if (sourceBefore.toString("utf8") !== AGENT_TASK_SOURCE_CONTENT) {
    fail("Agent Task source fixture does not match expected bytes");
  }
  const sourceDigestBefore = sha256Bytes(sourceBefore);
  const storageRegistryPath = path.join(HOME_DIR, ".infimount", "storages.json");
  const storageRegistryBefore = JSON.parse(fs.readFileSync(storageRegistryPath, "utf8"));
  if (storageRegistryBefore?.[0]?.config?.root !== "~") {
    fail("Agent Task regression requires the source storage to retain the legacy ~ root");
  }
  const storageRegistryDigestBefore = sha256Bytes(fs.readFileSync(storageRegistryPath));
  console.log("agent_task_legacy_home_source=yes");

  await driver.createSession();
  try {
    await driver.waitOperationalStorage(20000);
    const sourceCard = await driver.waitElement(
      "css selector",
      `[role="option"][aria-label="${AGENT_TASK_SOURCE_FILE}"]`,
      20000,
    );
    await driver.clickElement(sourceCard);
    await driver.clickText("Use with agent");
    await driver.waitText("Prepare Agent Task");

    await driver.setInput("#agent-task-objective", "Produce one deterministic reviewed result for publication safety validation.");
    await driver.setInput("#agent-task-outputs", "result.md");
    await driver.assertSelectPreselected('[aria-label="Local Agent Workspace"]', WRITABLE_WORKSPACE);

    await driver.clickText("Review scope", 30000);
    const preflight = await driver.waitElement(
      "css selector",
      '[data-testid="agent-task-preflight"]',
      30000,
    );
    const preflightText = await driver.text(preflight);
    if (!preflightText.includes("1") || !preflightText.includes("Prepared size")) {
      fail(`Agent Task preflight did not expose the expected single-file scope: ${preflightText}`);
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
      fail(`unexpected prepared Agent Task root: ${taskRoot}`);
    }
    const taskId = taskRoot.split("/").at(-1);
    await assertTaskScopedSidecarWorks(taskId);
    const hostTaskRoot = path.join(HOME_DIR, "agent-workspaces", "release-pilot-writable", taskRoot);
    const outputsRoot = path.join(hostTaskRoot, "outputs");
    fs.mkdirSync(outputsRoot, { recursive: true });
    const resultPath = path.join(outputsRoot, "result.md");
    fs.writeFileSync(resultPath, AGENT_TASK_OUTPUT_V1);

    await driver.clickText("Review outputs", 30000);
    await driver.waitElement("css selector", '[data-testid="agent-task-output-review"]', 30000);
    await driver.waitText(AGENT_TASK_OUTPUT_V1.trim(), 30000);
    await driver.waitText("0 of 1 selected", 30000);
    console.log("agent_task_nothing_selected_by_default=passed");

    await driver.clickCss('button[aria-label="Publish outputs/result.md"]');
    await driver.assertSelectPreselected('[aria-label="Publication destination storage"]', "Pilot Home");
    await driver.setInput("#agent-task-publication-dir", AGENT_TASK_DESTINATION_DIR);
    await driver.assertNoSelectOption(
      '[aria-label="Publication conflict policy"]',
      "overwrite",
      "Fail",
    );

    await driver.clickText("Review publication", 30000);
    let preview = await driver.waitElement(
      "css selector",
      '[data-testid="agent-task-publication-preview"]',
      30000,
    );
    let previewText = await driver.text(preview);
    if (!/Conflicts\s*1/i.test(previewText)) {
      fail(`fail-conflict preview did not report one conflict: ${previewText}`);
    }
    const blockedPublish = await driver.find(
      "xpath",
      "//button[contains(normalize-space(.), 'Publish 1 approved output')]",
      true,
    );
    if (blockedPublish) {
      fail("fail-conflict publication unexpectedly exposed an enabled publish action");
    }
    console.log("agent_task_fail_conflict_rejected=passed");
    console.log("agent_task_overwrite_unavailable=passed");

    await driver.selectOption('[aria-label="Publication conflict policy"]', "Keep both");
    await driver.clickText("Review publication", 30000);
    preview = await driver.waitElement(
      "css selector",
      '[data-testid="agent-task-publication-preview"]',
      30000,
    );
    await driver.waitFor(
      async () => {
        const text = await driver.text(preview);
        return /Rename\s*1/i.test(text) && /Conflicts\s*0/i.test(text);
      },
      30000,
      "rename publication preview",
    );

    // Invalidate the approved bytes after preview. The backend must rebuild
    // the current plan and reject this stale review rather than publishing.
    fs.writeFileSync(resultPath, AGENT_TASK_OUTPUT_V2);
    await driver.clickText("Publish 1 approved output", 30000);
    await driver.waitText("changed since it was reviewed", 30000);
    console.log("agent_task_stale_preview_rejected=passed");

    await driver.clickText("Refresh outputs", 30000);
    await driver.waitText(AGENT_TASK_OUTPUT_V2.trim(), 30000);
    await driver.clickCss('button[aria-label="Publish outputs/result.md"]');
    await driver.assertSelectPreselected('[aria-label="Publication destination storage"]', "Pilot Home");
    await driver.setInput("#agent-task-publication-dir", AGENT_TASK_DESTINATION_DIR);
    await driver.selectOption('[aria-label="Publication conflict policy"]', "Keep both");
    await driver.clickText("Review publication", 30000);
    await driver.waitFor(
      async () => {
        const current = await driver.find(
          "css selector",
          '[data-testid="agent-task-publication-preview"]',
          true,
        );
        if (!current) return false;
        const text = await driver.text(current);
        return /Rename\s*1/i.test(text) && /Conflicts\s*0/i.test(text);
      },
      30000,
      "fresh rename publication preview",
    );
    await driver.clickText("Publish 1 approved output", 30000);
    const success = await driver.waitElement(
      "css selector",
      '[data-testid="agent-task-publication-success"]',
      30000,
    );
    const successText = await driver.text(success);
    if (!successText.includes("Receipt:")) {
      fail(`publication success did not expose a receipt: ${successText}`);
    }
    console.log(`agent_task_id=${taskId}`);
    console.log("agent_task_publication_success=passed");
  } finally {
    await driver.quit();
  }

  const sourceAfter = fs.readFileSync(sourcePath);
  if (sha256Bytes(sourceAfter) !== sourceDigestBefore) {
    fail("Agent Task source bytes changed across prepare/review/publication");
  }
  const storageRegistryDigestAfter = sha256Bytes(fs.readFileSync(storageRegistryPath));
  if (storageRegistryDigestAfter !== storageRegistryDigestBefore) {
    fail("Agent Task flow changed the storage registry or MCP exposure");
  }
  console.log("agent_task_source_mcp_exposure_unchanged=passed");
  runState("assert-agent-task-publication");
}

async function main() {
  if (!HOME_DIR || HOME_DIR === path.parse(HOME_DIR).root) fail("unsafe pilot HOME");
  requireFile(APP_BINARY, "desktop application");
  requireFile(SIDECAR, "MCP sidecar");
  requireFile(STATE_SCRIPT, "release pilot state script");

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
    // Reproduce the real rc.13 pilot finding: browsing/operator construction accepts
    // the legacy home alias, so Agent Task preparation must accept the same storage
    // without requiring a manual root edit.
    runState("restore-legacy-home-alias");
    await agentTaskPublicationSession(driver);
    await guidedReenableSession(driver);
    runState("summary");
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
