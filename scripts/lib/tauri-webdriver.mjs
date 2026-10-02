#!/usr/bin/env node

import fs from "node:fs";
import path from "node:path";
import { setTimeout as sleep } from "node:timers/promises";

export const ELEMENT_KEY = "element-6066-11e4-a52e-4f735466cecf";

export function pilotFail(message) {
  throw new Error("RC10 real pilot WebDriver failed: " + message);
}

export function xpathLiteral(value) {
  if (!value.includes("'")) return "'" + value + "'";
  if (!value.includes('"')) return '"' + value + '"';
  const parts = value.split("'");
  return "concat(" + parts.map((part, index) => (index ? '"\'",' : "") + "'" + part + "'").join(",") + ")";
}

export function clickableTextXpath(text) {
  return "//*[self::button or @role='menuitem' or @role='option'][contains(normalize-space(.), " + xpathLiteral(text) + ")]";
}

export function textXpath(text) {
  return "//*[contains(normalize-space(.), " + xpathLiteral(text) + ")]";
}

export class TauriWebDriver {
  constructor(baseUrl) {
    this.baseUrl = baseUrl;
    this.sessionId = null;
  }

  async raw(method, endpoint, body) {
    const response = await fetch(this.baseUrl + endpoint, {
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

  async request(method, endpoint, body, options = {}) {
    const { response, payload } = await this.raw(method, endpoint, body);
    const value = payload && payload.value;
    const error = value && typeof value === "object" ? value.error : undefined;
    if (!response.ok || error) {
      if (options.optional && (error === "no such element" || response.status === 404)) return null;
      const detail =
        value && typeof value === "object"
          ? value.message || JSON.stringify(value)
          : JSON.stringify(payload);
      pilotFail(method + " " + endpoint + ": " + response.status + " " + detail);
    }
    return value ?? payload;
  }

  endpoint(suffix) {
    if (!this.sessionId) pilotFail("WebDriver session is not active");
    return "/session/" + this.sessionId + suffix;
  }

  elementId(value) {
    return value && (value[ELEMENT_KEY] || value.ELEMENT) || null;
  }

  async createSession(application) {
    const value = await this.request("POST", "/session", {
      capabilities: {
        alwaysMatch: {
          browserName: "wry",
          "tauri:options": { application },
        },
      },
    });
    this.sessionId = value && (value.sessionId || value["sessionId"]);
    if (!this.sessionId) pilotFail("WebDriver session id missing: " + JSON.stringify(value));
    await this.waitFor(
      async () => Boolean(await this.find("css selector", "body", true)),
      30000,
      "application body",
    );
  }

  async quit() {
    if (!this.sessionId) return;
    const id = this.sessionId;
    this.sessionId = null;
    try {
      await this.request("DELETE", "/session/" + id);
    } catch (error) {
      console.error(String(error));
    }
    await sleep(600);
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
    if (!id && !optional) pilotFail("element id missing for " + using + "=" + value);
    return id;
  }

  async findAll(using, value) {
    const result = await this.request("POST", this.endpoint("/elements"), { using, value });
    return Array.isArray(result) ? result.map((item) => this.elementId(item)).filter(Boolean) : [];
  }

  async clickElement(id) {
    await this.request("POST", this.endpoint("/element/" + id + "/click"), {});
  }

  async clearElement(id) {
    await this.request("POST", this.endpoint("/element/" + id + "/clear"), {});
  }

  async sendKeys(id, text) {
    await this.request("POST", this.endpoint("/element/" + id + "/value"), {
      text,
      value: [...text],
    });
  }

  async text(id) {
    return await this.request("GET", this.endpoint("/element/" + id + "/text"));
  }

  async attribute(id, name) {
    return await this.request(
      "GET",
      this.endpoint("/element/" + id + "/attribute/" + encodeURIComponent(name)),
    );
  }

  async displayed(id) {
    return await this.request("GET", this.endpoint("/element/" + id + "/displayed"));
  }

  async execute(script, args = []) {
    return await this.request("POST", this.endpoint("/execute/sync"), { script, args });
  }

  async screenshot(file) {
    const data = await this.request("GET", this.endpoint("/screenshot"));
    if (typeof data !== "string" || !data) pilotFail("WebDriver screenshot payload is missing");
    fs.mkdirSync(path.dirname(file), { recursive: true });
    fs.writeFileSync(file, Buffer.from(data, "base64"));
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
    pilotFail("timed out waiting for " + label);
  }

  async waitElement(using, value, timeoutMs = 15000) {
    return await this.waitFor(
      async () => await this.find(using, value, true),
      timeoutMs,
      using + "=" + value,
    );
  }

  async waitAbsent(using, value, timeoutMs = 10000) {
    await this.waitFor(
      async () => (await this.find(using, value, true)) === null,
      timeoutMs,
      "absence of " + using + "=" + value,
    );
  }

  async clickLocated(using, value, timeoutMs = 15000) {
    await this.waitFor(
      async () => {
        const id = await this.find(using, value, true);
        if (!id) return false;
        try {
          await this.execute(
            'const el=arguments[0]; if(el&&typeof el.scrollIntoView==="function"){el.scrollIntoView({block:"center",inline:"nearest"});} return true;',
            [{ [ELEMENT_KEY]: id }],
          );
          await sleep(50);
          await this.clickElement(id);
          return true;
        } catch (error) {
          const message = String(error && error.message || error);
          if (/element not interactable|stale element|click intercepted|no such element/i.test(message)) {
            return false;
          }
          throw error;
        }
      },
      timeoutMs,
      "clickable " + using + "=" + value,
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
          try {
            await this.execute(
              'const el=arguments[0]; el.scrollIntoView({block:"center",inline:"nearest"}); return true;',
              [{ [ELEMENT_KEY]: id }],
            );
            await sleep(50);
            await this.clickElement(id);
            return text;
          } catch {
            // reacquire below
          }
        }
      }
      await sleep(200);
    }
    pilotFail("timed out waiting for one of: " + texts.join(", "));
  }

  async bodySnapshot(maxChars = 4096) {
    const body = await this.find("css selector", "body", true);
    if (!body) return "<body unavailable>";
    const value = await this.text(body);
    return String(value || "").replace(/\s+/g, " ").trim().slice(0, maxChars);
  }

  async waitText(text, timeoutMs = 15000) {
    try {
      return await this.waitElement("xpath", textXpath(text), timeoutMs);
    } catch (error) {
      const snapshot = await this.bodySnapshot(8192).catch(() => "<snapshot unavailable>");
      throw new Error(String(error.message || error) + "\nDOM text snapshot: " + snapshot);
    }
  }

  async setInput(selector, value) {
    let id = await this.waitElement("css selector", selector);
    await this.clearElement(id);
    id = await this.waitElement("css selector", selector);
    await this.sendKeys(id, value);
  }

  async setInputXpath(xpath, value) {
    let id = await this.waitElement("xpath", xpath);
    await this.clearElement(id);
    id = await this.waitElement("xpath", xpath);
    await this.sendKeys(id, value);
  }

  async selectOption(triggerSelector, label) {
    await this.clickCss(triggerSelector);
    const option = await this.waitElement(
      "xpath",
      "//*[@role='option' and contains(normalize-space(.), " + xpathLiteral(label) + ")]",
    );
    await this.clickElement(option);
  }

  async assertSelectPreselected(triggerSelector, label) {
    await this.clickCss(triggerSelector);
    const option = await this.waitElement(
      "xpath",
      "//*[@role='option' and contains(normalize-space(.), " + xpathLiteral(label) + ")]",
    );
    const ariaSelected = await this.attribute(option, "aria-selected");
    const dataState = await this.attribute(option, "data-state");
    if (ariaSelected !== "true" && dataState !== "checked") {
      pilotFail(
        "select was not preselected to " + label +
        "; aria-selected=" + ariaSelected + " data-state=" + dataState,
      );
    }
    await this.clickElement(option);
  }

  async assertNoSelectOption(triggerSelector, forbiddenText, closeWithLabel) {
    await this.clickCss(triggerSelector);
    const forbidden = await this.find(
      "xpath",
      "//*[@role='option' and contains(translate(normalize-space(.), 'ABCDEFGHIJKLMNOPQRSTUVWXYZ', 'abcdefghijklmnopqrstuvwxyz'), " +
        xpathLiteral(forbiddenText.toLowerCase()) + ")]",
      true,
    );
    if (forbidden) pilotFail("select unexpectedly exposes forbidden option: " + forbiddenText);
    const close = await this.waitElement(
      "xpath",
      "//*[@role='option' and contains(normalize-space(.), " + xpathLiteral(closeWithLabel) + ")]",
    );
    await this.clickElement(close);
  }
}
