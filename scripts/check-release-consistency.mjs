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

const rawTag = process.env.GITHUB_REF_NAME || process.argv[2] || "";
if (!rawTag) fail("pass a tag/version argument or set GITHUB_REF_NAME");

const tag = rawTag.startsWith("v") ? rawTag : `v${rawTag}`;
const version = tag.slice(1);
if (!/^\d+\.\d+\.\d+(?:[-+][0-9A-Za-z.-]+)?$/.test(version)) {
  fail(`unsupported version format: ${version}`);
}

const packageVersion = json("apps/desktop/package.json").version;
const tauriVersion = json("apps/desktop/src-tauri/tauri.conf.json").version;
if (packageVersion !== version) fail(`apps/desktop/package.json version ${packageVersion} != ${version}`);
if (tauriVersion !== version) fail(`tauri.conf.json version ${tauriVersion} != ${version}`);

const cargoToml = read("apps/desktop/src-tauri/Cargo.toml");
if (!new RegExp(`^version\\s*=\\s*"${version.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")}"`, "m").test(cargoToml)) {
  fail(`apps/desktop/src-tauri/Cargo.toml must have version ${version}`);
}

const releaseNotesPath = `docs/release-notes-${version}.md`;
if (!fs.existsSync(releaseNotesPath)) fail(`${releaseNotesPath} must exist`);
contains(releaseNotesPath, `Infimount ${version}`);
contains(releaseNotesPath, `https://github.com/infimount/infimount/releases/tag/${tag}`);

contains("CHANGELOG.md", `## [${version}]`);
contains("CHANGELOG.md", `[Unreleased]: https://github.com/infimount/infimount/compare/${tag}...HEAD`);
contains("CHANGELOG.md", `[${version}]: https://github.com/infimount/infimount/compare/`);
contains("CHANGELOG.md", `...${tag}`);

contains("README.md", `Current stable release:** [${tag}]`);
contains("README.md", `INFIMOUNT_VERSION=${tag}`);

contains("docs/index.html", `"softwareVersion": "${version}"`);
contains("docs/index.html", `${tag} released`);
contains("docs/index.html", `releases/tag/${tag}`);
contains("docs/index.html", `INFIMOUNT_VERSION=${tag}`);
contains("docs/index.html", `Manual downloads for ${tag}`);

contains("docs/llms.txt", `Current stable release: ${tag}`);

console.log(`Release consistency check passed for ${tag}.`);
