#!/usr/bin/env node
import fs from "node:fs";

const raw = process.argv[2] ?? JSON.parse(fs.readFileSync("package.json", "utf8")).version;
if (typeof raw !== "string" || raw.includes("+")) {
  throw new Error(`unsupported release rehearsal version: ${String(raw)}`);
}

const match = raw.match(
  /^(\d+)\.(\d+)\.(\d+)(?:-([0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*))?$/,
);
if (!match) {
  throw new Error(`unsupported release rehearsal version: ${raw}`);
}

if (match[4]) {
  process.stdout.write(raw);
} else {
  const major = Number(match[1]);
  const minor = Number(match[2]);
  process.stdout.write(`${major}.${minor + 1}.0-rc.1`);
}
