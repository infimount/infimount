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
// Tauri rewrites Linux external binaries to use the bundled library directory.
// Apply the same deterministic RPATH before hashing so packaged verification
// validates the bytes that Tauri will embed.
if (target.includes("linux") && process.platform === "linux") {
  const patchelf = spawnSync("patchelf", ["--set-rpath", "$ORIGIN/../lib", destination], {
    cwd: root,
    stdio: "inherit",
  });
  if (patchelf.error) throw patchelf.error;
  if (patchelf.status !== 0) throw new Error(`patchelf exited with status ${patchelf.status}`);
}

const expectedVersion = JSON.parse(
  fs.readFileSync(path.join(root, "apps", "desktop", "package.json"), "utf8"),
).version;
const reportedVersion = run(destination, ["--version"], { capture: true });
if (reportedVersion !== `infimount_mcp ${expectedVersion}`) {
  throw new Error(`sidecar version mismatch: expected ${expectedVersion}, got ${reportedVersion}`);
}
const sha256 = crypto.createHash("sha256").update(fs.readFileSync(destination)).digest("hex");
const checksumLine = `${sha256}  ${path.basename(destination)}\n`;
fs.writeFileSync(`${destination}.sha256`, checksumLine, "utf8");
// Tauri packages this target-independent resource next to the external binary.
// It is regenerated for each single-target build job and is never trusted as a
// substitute for the signed release-level checksums.
fs.writeFileSync(path.join(destinationDir, "mcp.sha256"), checksumLine, "utf8");
console.log(`Prepared ${path.relative(root, destination)}`);
console.log(`SHA-256 ${sha256}`);
