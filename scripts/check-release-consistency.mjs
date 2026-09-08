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
  "docs/agent-tasks.md",
  "docs/recovery.md",
  "docs/troubleshooting.md",
  "docs/privacy.md",
  "docs/migration-v0.8.md",
  releaseNotesPath,
]) {
  if (!fs.existsSync(path)) fail(`${path} must exist`);
}
contains(releaseNotesPath, `Infimount ${version}`);
if (candidateChannel) {
  const releaseLink = `https://github.com/infimount/infimount/releases/tag/${tag}`;
  if (!read(releaseNotesPath).includes(releaseLink)) {
    contains(releaseNotesPath, "Release: not published yet.");
  }
} else {
  contains(releaseNotesPath, `https://github.com/infimount/infimount/releases/tag/${tag}`);
}

contains("CHANGELOG.md", `[Unreleased]: https://github.com/infimount/infimount/compare/v${coreVersion}...HEAD`);
contains("CHANGELOG.md", `[${coreVersion}]: https://github.com/infimount/infimount/compare/`);

const readme = read("README.md");
const stableMatch = readme.match(
  /\*\*Current stable release:\*\* \[v(\d+\.\d+\.\d+)\]\(https:\/\/github\.com\/infimount\/infimount\/releases\/tag\/v\1\)/,
);
if (!stableMatch) {
  fail("README.md must declare one canonical Current stable release link");
}
const publicStableVersion = stableMatch[1];
const candidateLine = readme.match(/\*\*Release candidate under validation:\*\* v(\d+\.\d+\.\d+) \(not published yet\)/);
const index = read("docs/index.html");
const llms = read("docs/llms.txt");

if (candidateChannel) {
  if (publicStableVersion === coreVersion) {
    fail(`candidate ${coreVersion} must not replace the public stable release before publication`);
  }
  contains("docs/index.html", `"softwareVersion": "${publicStableVersion}"`);
  contains("docs/llms.txt", `Current stable release: v${publicStableVersion}`);

  if (candidateLine && candidateLine[1] !== coreVersion) {
    fail(`README.md candidate ${candidateLine[1]} does not match ${coreVersion}`);
  }
  const llmsCandidate = llms.match(/Release candidate under validation: v(\d+\.\d+\.\d+) \(not published yet\)/);
  if (llmsCandidate && llmsCandidate[1] !== coreVersion) {
    fail(`docs/llms.txt candidate ${llmsCandidate[1]} does not match ${coreVersion}`);
  }
  const indexCandidate = index.match(/v(\d+\.\d+\.\d+) release candidate/);
  if (indexCandidate && indexCandidate[1] !== coreVersion) {
    fail(`docs/index.html candidate ${indexCandidate[1]} does not match ${coreVersion}`);
  }

  contains("CHANGELOG.md", `## [${coreVersion}] - Unreleased release candidate`);
} else {
  if (!new RegExp(`^## \\[${escapedVersion}\\] - \\d{4}-\\d{2}-\\d{2}$`, "m").test(read("CHANGELOG.md"))) {
    fail(`CHANGELOG.md must mark ${version} with a release date before the stable tag`);
  }
  if (publicStableVersion !== version) {
    fail(`README.md current stable v${publicStableVersion} != ${tag}`);
  }
  contains("docs/index.html", `"softwareVersion": "${version}"`);
  contains("docs/llms.txt", `Current stable release: ${tag}`);
  excludes("README.md", "not published yet");
  excludes("docs/index.html", "not published yet");
  excludes("docs/llms.txt", "not published yet");
}

const msrv = workspaceCargoToml.match(/^rust-version\s*=\s*"([^"]+)"/m)?.[1];
if (!msrv) fail("Cargo.toml workspace rust-version is missing");
contains("README.md", `Rust ${msrv}+`);

console.log(`Release consistency check passed for ${tag} (${candidateChannel ? "candidate" : "stable"}).`);
