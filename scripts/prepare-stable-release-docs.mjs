#!/usr/bin/env node
import fs from "node:fs";

const tag = process.argv[2] ?? process.env.GITHUB_REF_NAME ?? "";
if (!/^v\d+\.\d+\.\d+$/.test(tag)) {
  throw new Error("pass a stable tag such as v0.8.0 (prerelease/build metadata is not allowed)");
}
const version = tag.slice(1);
const date = process.env.RELEASE_DATE ?? new Date().toISOString().slice(0, 10);
const replace = (path, pattern, replacement) => {
  const input = fs.readFileSync(path, "utf8");
  const output = input.replace(pattern, replacement);
  if (output === input) throw new Error(`${path}: expected release-candidate text was not found`);
  fs.writeFileSync(path, output, "utf8");
};

replace(
  "README.md",
  /\*\*Current stable release:\*\* \[v0\.7\.1\]\([^\n]+\)\n\n\*\*Release candidate under validation:\*\* v[^\n]+\n/,
  `**Current stable release:** [${tag}](https://github.com/infimount/infimount/releases/tag/${tag})\n`,
);
replace(
  "CHANGELOG.md",
  new RegExp(`^## \\[${version.replaceAll(".", "\\.")}\\] - Unreleased release candidate$`, "m"),
  `## [${version}] - ${date}`,
);
replace("docs/index.html", '"softwareVersion": "0.7.1"', `"softwareVersion": "${version}"`);
replace("docs/index.html", `v${version} release candidate`, `${tag} stable`);
replace("docs/index.html", `>${tag} release page after publication<`, `>${tag} release page<`);
replace(
  "docs/index.html",
  new RegExp(`After the v${version.replaceAll(".", "\\.")} release candidate is published, pin it with <code>INFIMOUNT_VERSION=v${version.replaceAll(".", "\\.")}</code>\\. Until then, the command installs the current stable release\\.`),
  `Pin this stable release with <code>INFIMOUNT_VERSION=${tag}</code>.`,
);
replace(
  "docs/index.html",
  new RegExp(`Manual downloads for the v${version.replaceAll(".", "\\.")} release candidate appear only after publication\\.`),
  `The downloads below resolve to the published ${tag} stable release.`,
);
replace(
  "docs/llms.txt",
  /- Current stable release: v0\.7\.1\n- Release candidate under validation: v[^\n]+\n/,
  `- Current stable release: ${tag}\n`,
);
replace(
  "docs/llms.txt",
  `The v${version} release candidate adds`,
  `${tag} adds`,
);

console.log(`Prepared stable public documentation for ${tag} (${date}).`);
