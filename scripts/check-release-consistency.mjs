#!/usr/bin/env node
import fs from "node:fs";

const fail = (message) => {
  console.error(`release consistency check failed: ${message}`);
  process.exit(1);
};
const read = (path) => fs.readFileSync(path, "utf8");
const json = (path) => JSON.parse(read(path));
const contains = (path, needle) => {
  if (!read(path).includes(needle)) fail(`${path} must contain: ${needle}`);
};
const excludes = (path, needle) => {
  if (read(path).includes(needle)) fail(`${path} must not contain: ${needle}`);
};

const rawTag = process.argv[2] || process.env.GITHUB_REF_NAME || "";
if (!rawTag) fail("pass a tag/version argument or set GITHUB_REF_NAME");
const tag = rawTag.startsWith("v") ? rawTag : `v${rawTag}`;
const version = tag.slice(1);
if (version.includes("+")) fail("SemVer build metadata is not supported for release tags");
if (!/^\d+\.\d+\.\d+(?:-[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?$/.test(version)) {
  fail(`unsupported version format: ${version}`);
}
const prerelease = version.includes("-");
const unpublishedCandidate = process.env.INFIMOUNT_UNPUBLISHED_CANDIDATE === "1" && !prerelease;
const candidateChannel = prerelease || unpublishedCandidate;
const coreVersion = version.split("-", 1)[0];

for (const [path, actual] of [
  ["package.json", json("package.json").version],
  ["apps/desktop/package.json", json("apps/desktop/package.json").version],
  ["apps/desktop/src-tauri/tauri.conf.json", json("apps/desktop/src-tauri/tauri.conf.json").version],
]) {
  if (actual !== version) fail(`${path} version ${actual} != ${version}; run sync-release-version after checkout`);
}

const workspaceCargoToml = read("Cargo.toml");
const escapedVersion = version.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
if (!new RegExp(`^version\\s*=\\s*"${escapedVersion}"`, "m").test(workspaceCargoToml)) {
  fail(`workspace Cargo.toml version must be ${version}`);
}
const desktopCargo = read("apps/desktop/src-tauri/Cargo.toml");
if (!/^version\.workspace\s*=\s*true\s*$/m.test(desktopCargo)) {
  fail("desktop Cargo.toml must inherit the workspace version");
}

const releaseNotesPath = `docs/release-notes-${version}.md`;
for (const path of [
  "docs/agent-workspaces.md",
  "docs/recovery.md",
  "docs/troubleshooting.md",
  "docs/privacy.md",
  "docs/migration-v0.8.md",
  releaseNotesPath,
]) {
  if (!fs.existsSync(path)) fail(`${path} must exist`);
}
contains(releaseNotesPath, `Infimount ${version}`);
contains(releaseNotesPath, `https://github.com/infimount/infimount/releases/tag/${tag}`);

contains("CHANGELOG.md", `[Unreleased]: https://github.com/infimount/infimount/compare/v${coreVersion}...HEAD`);
contains("CHANGELOG.md", `[${coreVersion}]: https://github.com/infimount/infimount/compare/`);

if (candidateChannel) {
  contains("CHANGELOG.md", `## [${coreVersion}] - Unreleased release candidate`);
  contains("README.md", "Current stable release:** [v0.7.1]");
  contains("README.md", `Release candidate under validation:** v${coreVersion} (not published yet)`);
  contains("docs/index.html", '"softwareVersion": "0.7.1"');
  contains("docs/index.html", `v${coreVersion} release candidate`);
  contains("docs/llms.txt", "Current stable release: v0.7.1");
  contains("docs/llms.txt", `Release candidate under validation: v${coreVersion} (not published yet)`);
} else {
  if (!new RegExp(`^## \\[${escapedVersion}\\] - \\d{4}-\\d{2}-\\d{2}$`, "m").test(read("CHANGELOG.md"))) {
    fail(`CHANGELOG.md must mark ${version} with a release date before the stable tag`);
  }
  contains("README.md", `Current stable release:** [${tag}](https://github.com/infimount/infimount/releases/tag/${tag})`);
  excludes("README.md", "not published yet");
  contains("docs/index.html", `"softwareVersion": "${version}"`);
  excludes("docs/index.html", "release candidate appear only after publication");
  contains("docs/llms.txt", `Current stable release: ${tag}`);
  excludes("docs/llms.txt", "not published yet");
}

const msrv = workspaceCargoToml.match(/^rust-version\s*=\s*"([^"]+)"/m)?.[1];
if (!msrv) fail("Cargo.toml workspace rust-version is missing");
contains("README.md", `Rust ${msrv}+`);

console.log(`Release consistency check passed for ${tag} (${candidateChannel ? "candidate" : "stable"}).`);
