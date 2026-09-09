#!/usr/bin/env node
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const derive = (version) =>
  execFileSync("node", [path.join(root, "scripts", "derive-release-rehearsal-version.mjs"), version], {
    cwd: root,
    encoding: "utf8",
  });

assert.equal(derive("0.8.0"), "0.9.0-rc.1");
assert.equal(derive("0.9.0-rc.1"), "0.9.0-rc.1");
assert.equal(derive("0.9.0-beta.2"), "0.9.0-beta.2");
assert.equal(derive("1.12.3"), "1.13.0-rc.1");
assert.throws(() => derive("0.9.0+build.1"));
assert.throws(() => derive("not-semver"));

console.log("Release rehearsal version derivation check passed.");
