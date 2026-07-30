#!/usr/bin/env node
import crypto from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { spawnSync } from "node:child_process";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const args = process.argv.slice(2);
let target;
for (let index = 0; index < args.length; index += 1) {
  if (args[index] === "--target") {
    target = args[index + 1];
    if (!target) throw new Error("--target requires a Rust target triple");
    index += 1;
  } else {
    throw new Error(`unsupported argument: ${args[index]}`);
  }
}

const run = (command, commandArgs, options = {}) => {
  const result = spawnSync(command, commandArgs, {
    cwd: root,
    encoding: "utf8",
    stdio: options.capture ? "pipe" : "inherit",
    ...options,
  });
  if (result.error) throw result.error;
  if (result.status !== 0) {
    throw new Error(`${command} exited with status ${result.status}`);
  }
  return (result.stdout || "").trim();
};

if (!target) {
  target = run("rustc", ["--print", "host-tuple"], { capture: true });
}
if (!/^[A-Za-z0-9_.-]+$/.test(target)) throw new Error(`invalid Rust target triple: ${target}`);

run("cargo", [
  "build",
  "--release",
  "-p",
  "infimount_mcp",
  "--bin",
  "infimount_mcp",
  "--target",
  target,
]);

const executableSuffix = target.includes("windows") ? ".exe" : "";
const source = path.join(root, "target", target, "release", `infimount_mcp${executableSuffix}`);
const destinationDir = path.join(root, "apps", "desktop", "src-tauri", "binaries");
const destination = path.join(destinationDir, `mcp-${target}${executableSuffix}`);
if (!fs.statSync(source).isFile()) throw new Error(`sidecar was not built at ${source}`);
fs.mkdirSync(destinationDir, { recursive: true });
fs.copyFileSync(source, destination);
if (!executableSuffix) fs.chmodSync(destination, 0o755);

const expectedVersion = JSON.parse(
  fs.readFileSync(path.join(root, "apps", "desktop", "package.json"), "utf8"),
).version;
const reportedVersion = run(destination, ["--version"], { capture: true });
if (reportedVersion !== `infimount_mcp ${expectedVersion}`) {
  throw new Error(`sidecar version mismatch: expected ${expectedVersion}, got ${reportedVersion}`);
}
const sha256 = crypto.createHash("sha256").update(fs.readFileSync(destination)).digest("hex");
fs.writeFileSync(`${destination}.sha256`, `${sha256}  ${path.basename(destination)}\n`, "utf8");
console.log(`Prepared ${path.relative(root, destination)}`);
console.log(`SHA-256 ${sha256}`);
