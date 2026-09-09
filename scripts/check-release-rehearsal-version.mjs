#!/usr/bin/env node
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const script = path.join(root, "scripts", "derive-release-rehearsal-version.mjs");
const derive = (version) =>
  execFileSync("node", [script, ...(version === undefined ? [] : [version])], {
    cwd: os.tmpdir(),
    encoding: "utf8",
  });

assert.equal(derive("0.8.0"), "0.8.1-rc.1");
assert.equal(derive("0.8.1-rc.1"), "0.8.1-rc.1");
assert.equal(derive("0.8.1-beta.2"), "0.8.1-beta.2");
assert.equal(derive("1.12.3"), "1.12.4-rc.1");
assert.throws(() => derive("0.8.1+build.1"));
assert.throws(() => derive("not-semver"));

const packageVersion = JSON.parse(fs.readFileSync(path.join(root, "package.json"), "utf8")).version;
assert.equal(derive(), derive(packageVersion));

console.log("Release rehearsal version derivation check passed.");
