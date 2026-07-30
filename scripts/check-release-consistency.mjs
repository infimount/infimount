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

const rawTag = process.argv[2] || process.env.GITHUB_REF_NAME || "";
if (!rawTag) fail("pass a tag/version argument or set GITHUB_REF_NAME");

const tag = rawTag.startsWith("v") ? rawTag : `v${rawTag}`;
const version = tag.slice(1);
if (!/^\d+\.\d+\.\d+(?:[-+][0-9A-Za-z.-]+)?$/.test(version)) {
  fail(`unsupported version format: ${version}`);
}

const rootPackageVersion = json("package.json").version;
const packageVersion = json("apps/desktop/package.json").version;
const tauriVersion = json("apps/desktop/src-tauri/tauri.conf.json").version;
if (rootPackageVersion !== version) fail(`package.json version ${rootPackageVersion} != ${version}`);
if (packageVersion !== version) fail(`apps/desktop/package.json version ${packageVersion} != ${version}`);
if (tauriVersion !== version) fail(`tauri.conf.json version ${tauriVersion} != ${version}`);

const cargoToml = read("apps/desktop/src-tauri/Cargo.toml");
const escapedVersion = version.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
const hasLiteralVersion = new RegExp(`^version\\s*=\\s*"${escapedVersion}"`, "m").test(cargoToml);
const inheritsWorkspaceVersion = /^version\.workspace\s*=\s*true\s*$/m.test(cargoToml);
if (!hasLiteralVersion && !inheritsWorkspaceVersion) {
  fail(`apps/desktop/src-tauri/Cargo.toml must declare version ${version} or inherit it from the workspace`);
}
if (inheritsWorkspaceVersion) {
  const workspaceCargoToml = read("Cargo.toml");
  if (!new RegExp(`^version\\s*=\\s*"${escapedVersion}"`, "m").test(workspaceCargoToml)) {
    fail(`workspace Cargo.toml version must be ${version}`);
  }
}

const releaseNotesPath = `docs/release-notes-${version}.md`;
const requiredV08Docs = [
  "docs/agent-workspaces.md",
  "docs/recovery.md",
  "docs/troubleshooting.md",
  "docs/privacy.md",
  "docs/migration-v0.8.md",
  releaseNotesPath,
];
for (const path of requiredV08Docs) {
  if (!fs.existsSync(path)) fail(`${path} must exist`);
}
contains(releaseNotesPath, `Infimount ${version}`);
contains(releaseNotesPath, `https://github.com/infimount/infimount/releases/tag/${tag}`);

contains("CHANGELOG.md", `## [${version}] - Unreleased release candidate`);
contains("CHANGELOG.md", `[Unreleased]: https://github.com/infimount/infimount/compare/${tag}...HEAD`);
contains("CHANGELOG.md", `[${version}]: https://github.com/infimount/infimount/compare/`);
contains("CHANGELOG.md", `...${tag}`);

contains("README.md", "Current stable release:** [v0.7.1]");
contains("README.md", `Release candidate under validation:** ${tag} (not published yet)`);
contains("README.md", `INFIMOUNT_VERSION=${tag}`);

contains("docs/index.html", '"softwareVersion": "0.7.1"');
contains("docs/index.html", `${tag} release candidate`);
contains("docs/index.html", `releases/tag/${tag}`);
contains("docs/index.html", `INFIMOUNT_VERSION=${tag}`);
contains("docs/index.html", `Manual downloads for the ${tag} release candidate appear only after publication`);

contains("docs/llms.txt", "Current stable release: v0.7.1");
contains("docs/llms.txt", `Release candidate under validation: ${tag} (not published yet)`);

const workspaceCargoToml = read("Cargo.toml");
const msrv = workspaceCargoToml.match(/^rust-version\s*=\s*"([^"]+)"/m)?.[1];
if (!msrv) fail("Cargo.toml workspace rust-version is missing");
contains("README.md", `Rust ${msrv}+`);

console.log(`Release consistency check passed for ${tag}.`);
