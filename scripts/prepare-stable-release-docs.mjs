#!/usr/bin/env node
import fs from "node:fs";

const tag = process.argv[2] ?? process.env.GITHUB_REF_NAME ?? "";
if (!/^v\d+\.\d+\.\d+$/.test(tag)) {
  throw new Error("pass a stable tag such as v0.9.0 (prerelease/build metadata is not allowed)");
}
const version = tag.slice(1);
const date = process.env.RELEASE_DATE ?? new Date().toISOString().slice(0, 10);
const releaseUrl = `https://github.com/infimount/infimount/releases/tag/${tag}`;

const read = (path) => fs.readFileSync(path, "utf8");
const write = (path, value) => fs.writeFileSync(path, value, "utf8");
const replaceRequired = (path, pattern, replacement, description) => {
  const input = read(path);
  const output = input.replace(pattern, replacement);
  if (output === input) throw new Error(`${path}: ${description} was not found`);
  write(path, output);
};
const replaceOptional = (path, pattern, replacement) => {
  const input = read(path);
  const output = input.replace(pattern, replacement);
  if (output !== input) write(path, output);
};
const replaceAllOptional = (path, replacements) => {
  let output = read(path);
  for (const [pattern, replacement] of replacements) {
    output =
      typeof pattern === "string"
        ? output.split(pattern).join(replacement)
        : output.replace(pattern, replacement);
  }
  write(path, output);
};

const readme = read("README.md");
const stableMatch = readme.match(
  /\*\*Current stable release:\*\* \[v(\d+\.\d+\.\d+)\]\(https:\/\/github\.com\/infimount\/infimount\/releases\/tag\/v\1\)/,
);
if (!stableMatch) throw new Error("README.md: canonical Current stable release line was not found");
const previousVersion = stableMatch[1];
if (previousVersion === version) throw new Error(`${tag} is already the public stable release`);

const releaseNotesPath = `docs/release-notes-${version}.md`;
if (!fs.existsSync(releaseNotesPath)) {
  throw new Error(`${releaseNotesPath}: stable release notes must exist before publication`);
}

replaceRequired(
  "README.md",
  /\*\*Current stable release:\*\* \[v\d+\.\d+\.\d+\]\(https:\/\/github\.com\/infimount\/infimount\/releases\/tag\/v\d+\.\d+\.\d+\)/,
  `**Current stable release:** [${tag}](${releaseUrl})`,
  "current stable release line",
);
replaceOptional(
  "README.md",
  /\n\*\*Release candidate under validation:\*\* v\d+\.\d+\.\d+ \(not published yet\)\n/,
  "\n",
);
replaceOptional(
  "README.md",
  new RegExp(`INFIMOUNT_VERSION=v${previousVersion.replaceAll(".", "\\.")}`, "g"),
  `INFIMOUNT_VERSION=${tag}`,
);

replaceRequired(
  "CHANGELOG.md",
  new RegExp(`^## \\[${version.replaceAll(".", "\\.")}\\] - Unreleased release candidate$`, "m"),
  `## [${version}] - ${date}`,
  `${version} release-candidate changelog heading`,
);

replaceRequired(
  "docs/index.html",
  `"softwareVersion": "${previousVersion}"`,
  `"softwareVersion": "${version}"`,
  "previous stable softwareVersion",
);
for (const [pattern, replacement] of [
  [`<p class="release-label">v${previousVersion} stable · open source</p>`, `<p class="release-label">${tag} stable · open source</p>`],
  [`>Download v${previousVersion}<`, `>Download ${tag}<`],
  [`<p class="kicker">Install v${previousVersion}</p>`, `<p class="kicker">Install ${tag}</p>`],
  [`href="https://github.com/infimount/infimount/releases/tag/v${previousVersion}">Open the v${previousVersion} release<`, `href="${releaseUrl}">Open the ${tag} release<`],
]) {
  replaceOptional("docs/index.html", pattern, replacement);
}
replaceOptional(
  "docs/index.html",
  new RegExp(`v${version.replaceAll(".", "\\.")} release candidate`, "g"),
  `${tag} stable`,
);
replaceOptional(
  "docs/index.html",
  new RegExp(`INFIMOUNT_VERSION=v${previousVersion.replaceAll(".", "\\.")}`, "g"),
  `INFIMOUNT_VERSION=${tag}`,
);

replaceRequired(
  "docs/llms.txt",
  new RegExp(`Current stable release: v${previousVersion.replaceAll(".", "\\.")}`),
  `Current stable release: ${tag}`,
  "previous stable release line",
);
replaceOptional(
  "docs/llms.txt",
  /\n- Release candidate under validation: v\d+\.\d+\.\d+ \(not published yet\)/,
  "",
);

const releaseStateReplacements = [
  [
    `Agent Tasks are implemented on \`main\` and targeted for v${version}. The current v${previousVersion} stable release does not include this workflow.`,
    `Agent Tasks are included in ${tag}.`,
  ],
  [
    `The complete implementation is on \`main\` and is targeted for **v${version}**. The current v${previousVersion} stable release does not include this workflow.`,
    `The complete implementation is included in **${tag}**.`,
  ],
  [
    `Agent Tasks on \`main\`, targeted for v${version}`,
    `Agent Tasks in ${tag}`,
  ],
  [
    `Agent Tasks on main, targeted for v${version}`,
    `Agent Tasks in ${tag}`,
  ],
  [`targeted for **v${version}**`, `included in **${tag}**`],
  [`targeted for v${version}`, `included in ${tag}`],
  [
    `The current v${previousVersion} stable release does not include this workflow.`,
    `This workflow is included in ${tag}.`,
  ],
  [`pilot evidence remains the v${version} release gate`, `pilot evidence completed for ${tag}`],
];
for (const path of ["README.md", "docs/agent-tasks.md", "docs/llms.txt"]) {
  if (fs.existsSync(path)) replaceAllOptional(path, releaseStateReplacements);
}

replaceRequired(
  releaseNotesPath,
  "Release: not published yet.",
  `Release: ${releaseUrl}`,
  "unpublished release marker",
);
replaceOptional(
  releaseNotesPath,
  /^> .*not been published[^\n]*\n\n?/m,
  "",
);

console.log(`Prepared stable public documentation for ${tag} (${date}); previous stable was v${previousVersion}.`);
