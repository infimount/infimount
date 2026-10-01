#!/usr/bin/env node

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
          return snapshot.includes("Pilot Home");
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
    await driver.waitText("pilot-browser.txt", 20000);
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

  const workspaceSelect = await driver.waitElement(
    "css selector",
    '[aria-label="Agent Access workspace"]',
  );
  const selectedText = await driver.text(workspaceSelect);
  if (!selectedText.includes(expectedName)) {
    fail(`Connect agent did not preselect ${expectedName}; selected=${selectedText}`);
  }

  await driver.clickText("Prepare agent access", 30000);
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
    await driver.waitText("pilot-browser.txt", 20000);
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
