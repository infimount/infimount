#!/usr/bin/env node
// Contract-fixture checks for the deliberately small JSON Schema keyword
// subset in docs/schemas/agent-task-execution-v1.schema.json.
// This is not a general JSON Schema engine. Authoritative runtime and
// cross-process state-transition validation must be implemented in Phase A2.
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const fixtureDir = path.join(root, "tests/fixtures/agent-task-executions");
const load = (file) => JSON.parse(fs.readFileSync(file, "utf8"));
const schema = load(path.join(root, "docs/schemas/agent-task-execution-v1.schema.json"));
const errorsFor = (value, rule, label = "$") => {
  const errors = [];
  if (Object.hasOwn(rule, "oneOf")) {
    const matches = rule.oneOf.filter((candidate) => errorsFor(value, candidate, label).length === 0);
    if (matches.length !== 1) errors.push(`${label}: expected exactly one schema match`);
    return errors;
  }
  if (Object.hasOwn(rule, "const") && value !== rule.const) {
    errors.push(`${label}: invalid fixed value`);
  }
  if (Array.isArray(rule.enum) && !rule.enum.includes(value)) {
    errors.push(`${label}: unsupported enumerated value`);
  }
  if (rule.type !== undefined) {
    const validType =
      rule.type === "null" ? value === null :
      rule.type === "integer" ? Number.isSafeInteger(value) :
      rule.type === "array" ? Array.isArray(value) :
      rule.type === "object" ? value !== null && typeof value === "object" && !Array.isArray(value) :
      rule.type === "string" ? typeof value === "string" :
      false;
    if (!validType) {
      errors.push(`${label}: expected ${rule.type}`);
      return errors;
    }
  }
  if (typeof value === "string") {
    if (rule.minLength !== undefined && value.length < rule.minLength) errors.push(`${label}: too short`);
    if (rule.maxLength !== undefined && value.length > rule.maxLength) errors.push(`${label}: too long`);
    if (rule.pattern !== undefined && !new RegExp(rule.pattern, "u").test(value)) errors.push(`${label}: invalid pattern`);
    if (rule.format === "date-time") {
      if (!/^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d+)?(?:Z|[+-]\d{2}:\d{2})$/.test(value) || !Number.isFinite(Date.parse(value))) {
        errors.push(`${label}: invalid RFC3339 timestamp`);
      }
    }
  }
  if (typeof value === "number") {
    if (rule.minimum !== undefined && value < rule.minimum) errors.push(`${label}: below minimum`);
    if (rule.maximum !== undefined && value > rule.maximum) errors.push(`${label}: above maximum`);
  }
  if (Array.isArray(value)) {
    if (rule.maxItems !== undefined && value.length > rule.maxItems) errors.push(`${label}: too many entries`);
    if (rule.uniqueItems && new Set(value.map((item) => JSON.stringify(item))).size !== value.length) errors.push(`${label}: duplicate entry`);
    if (rule.items) for (let i = 0; i < value.length; i++) errors.push(...errorsFor(value[i], rule.items, `${label}[${i}]`));
  }
  if (value !== null && typeof value === "object" && !Array.isArray(value)) {
    for (const key of rule.required ?? []) {
      if (!Object.hasOwn(value, key)) errors.push(`${label}: missing ${key}`);
    }
    if (rule.additionalProperties === false) {
      for (const key of Object.keys(value)) {
        if (!Object.hasOwn(rule.properties ?? {}, key)) errors.push(`${label}: forbidden ${key}`);
      }
    }
    for (const [key, childRule] of Object.entries(rule.properties ?? {})) {
      if (Object.hasOwn(value, key)) errors.push(...errorsFor(value[key], childRule, `${label}.${key}`));
    }
    for (const conditional of rule.allOf ?? []) {
      if (!conditional.if || errorsFor(value, conditional.if, label).length === 0) {
        if (conditional.then) errors.push(...errorsFor(value, conditional.then, label));
      }
    }
  }
  return errors;
};

assert.equal(schema.$schema, "https://json-schema.org/draft/2020-12/schema");
assert.equal(schema.type, "object");
assert.equal(schema.additionalProperties, false);
assert.equal(schema.properties.schemaVersion.const, 1);
assert.deepEqual(new Set(schema.required), new Set(Object.keys(schema.properties)));
assert.equal(schema.properties.enabledTaskTools.uniqueItems, true);
assert.equal(schema.properties.enabledTaskTools.maxItems, 6);
assert.deepEqual(new Set(schema.properties.enabledTaskTools.items.enum),
  new Set(["list_dir", "stat_path", "read_file", "search_paths", "write_file", "mkdir"]));
assert.equal(schema.properties.outputTotalBytes.maximum, 2147483648);

const valid = [
  "valid-local-prepared.json",
  "valid-local-succeeded.json",
  "valid-remote-prepared.json"
];
const invalid = [
  "invalid-sensitive-source-path.json",
  "invalid-ambient-tool.json",
  "invalid-terminal-state.json",
  "invalid-remote-local-executable.json",
  "invalid-output-inconsistent.json"
];
for (const name of valid) {
  const errors = errorsFor(load(path.join(fixtureDir, name)), schema);
  assert.deepEqual(errors, [], `expected valid fixture ${name}: ${errors.join(", ")}`);
}
for (const name of invalid) {
  const errors = errorsFor(load(path.join(fixtureDir, name)), schema);
  assert.ok(errors.length > 0, `unsafe fixture unexpectedly accepted: ${name}`);
}

const transitions = {
  prepared: new Set(["running", "failed", "cancelled"]),
  running: new Set(["succeeded", "failed", "cancelled"]),
  succeeded: new Set(),
  failed: new Set(),
  cancelled: new Set()
};
assert.equal(transitions.prepared.has("running"), true);
assert.equal(transitions.running.has("succeeded"), true);
for (const state of ["succeeded", "failed", "cancelled"]) {
  assert.equal(transitions[state].size, 0, `${state} must be terminal`);
}
assert.equal(transitions.prepared.has("succeeded"), false, "cannot succeed without running");

console.log("Agent Task execution v1 design-contract fixtures passed (3 accepted, 5 rejected).");
