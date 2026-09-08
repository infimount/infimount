#!/usr/bin/env node
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const temp = fs.mkdtempSync(path.join(os.tmpdir(), "infimount-release-doc-tooling-"));

const write = (relativePath, content) => {
  const target = path.join(temp, relativePath);
  fs.mkdirSync(path.dirname(target), { recursive: true });
  fs.writeFileSync(target, content, "utf8");
};

const writeVersionFiles = (version) => {
  write("package.json", `${JSON.stringify({ version }, null, 2)}\n`);
  write("apps/desktop/package.json", `${JSON.stringify({ version }, null, 2)}\n`);
  write(
    "apps/desktop/src-tauri/tauri.conf.json",
    `${JSON.stringify({ version, bundle: {} }, null, 2)}\n`,
  );
  write(
    "Cargo.toml",
    `[workspace.package]\nversion = "${version}"\nrust-version = "1.94"\n`,
  );
  write("apps/desktop/src-tauri/Cargo.toml", "[package]\nversion.workspace = true\n");
};

const run = (script, args = [], env = {}) =>
  execFileSync("node", [path.join(root, "scripts", script), ...args], {
    cwd: temp,
    env: { ...process.env, ...env },
    stdio: "pipe",
  });

try {
  const stable = "1.2.3";
  const core = "1.3.0";
  const candidate = `${core}-rc.1`;

  writeVersionFiles(candidate);
  write(
    "README.md",
    `# Fixture\n\n**Current stable release:** [v${stable}](https://github.com/infimount/infimount/releases/tag/v${stable})\n\nInstall with INFIMOUNT_VERSION=v${stable}.\n\nRust 1.94+\n`,
  );
  write(
    "CHANGELOG.md",
    `# Changelog\n\n## [Unreleased]\n\n## [${core}] - Unreleased release candidate\n\n### Added\n\n- Future feature.\n\n[Unreleased]: https://github.com/infimount/infimount/compare/v${core}...HEAD\n[${core}]: https://github.com/infimount/infimount/compare/v${stable}...v${core}\n`,
  );
  write(
    "docs/index.html",
    `<!doctype html><meta name="fixture"><script type="application/ld+json">{"softwareVersion": "${stable}"}</script><p class="release-label">v${stable} stable · open source</p><a>Download v${stable}</a><p class="kicker">Install v${stable}</p><a href="https://github.com/infimount/infimount/releases/tag/v${stable}">Open the v${stable} release</a>`,
  );
  write("docs/llms.txt", `# Fixture\n\n- Current stable release: v${stable}\n`);
  for (const doc of [
    "agent-workspaces.md",
    "agent-tasks.md",
    "recovery.md",
    "troubleshooting.md",
    "privacy.md",
    "migration-v0.8.md",
  ]) {
    write(`docs/${doc}`, `# ${doc}\n`);
  }
  write(
    `docs/release-notes-${candidate}.md`,
    `# Infimount ${candidate}: Fixture\n\nRelease: not published yet.\n`,
  );

  run("check-release-consistency.mjs", [`v${candidate}`]);

  writeVersionFiles(core);
  write(
    `docs/release-notes-${core}.md`,
    `# Infimount ${core}: Fixture\n\n> ${core} has not been published yet.\n\nRelease: not published yet.\n`,
  );
  run("prepare-stable-release-docs.mjs", [`v${core}`], { RELEASE_DATE: "2030-01-02" });
  run("check-release-consistency.mjs", [`v${core}`]);

  const readme = fs.readFileSync(path.join(temp, "README.md"), "utf8");
  const changelog = fs.readFileSync(path.join(temp, "CHANGELOG.md"), "utf8");
  const index = fs.readFileSync(path.join(temp, "docs/index.html"), "utf8");
  const llms = fs.readFileSync(path.join(temp, "docs/llms.txt"), "utf8");
  const notes = fs.readFileSync(path.join(temp, `docs/release-notes-${core}.md`), "utf8");

  assert.match(readme, /Current stable release:\*\* \[v1\.3\.0\]/);
  assert.match(readme, /INFIMOUNT_VERSION=v1\.3\.0/);
  assert.doesNotMatch(readme, /Current stable release:\*\* \[v1\.2\.3\]/);
  assert.match(changelog, /## \[1\.3\.0\] - 2030-01-02/);
  assert.match(index, /"softwareVersion": "1\.3\.0"/);
  assert.match(llms, /Current stable release: v1\.3\.0/);
  assert.match(notes, /releases\/tag\/v1\.3\.0/);
  assert.doesNotMatch(notes, /not published yet/);
} finally {
  fs.rmSync(temp, { recursive: true, force: true });
}

console.log("Release documentation tooling check passed across a synthetic future release.");
