#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"
EXPECTED_VERSION="${EXPECTED_VERSION:-}"
if [[ -z "$EXPECTED_VERSION" ]]; then
  EXPECTED_VERSION="$(node -p "require('./apps/desktop/package.json').version")"
fi
BINARY="${1:-}"

if [[ -z "$BINARY" ]]; then
  TARGET="${TARGET:-$(rustc --print host-tuple)}"
  EXT=""
  [[ "$TARGET" == *windows* ]] && EXT=".exe"
  BINARY="$ROOT/apps/desktop/src-tauri/binaries/mcp-${TARGET}${EXT}"
fi
[[ -x "$BINARY" || ( "$BINARY" == *.exe && -f "$BINARY" ) ]] || { echo "Sidecar is missing or not executable: $BINARY" >&2; exit 1; }

BINARY="$BINARY" EXPECTED_VERSION="$EXPECTED_VERSION" node --input-type=module <<'NODE'
import crypto from "node:crypto";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { execFileSync } from "node:child_process";
const binary = process.env.BINARY;
const expected = process.env.EXPECTED_VERSION;
const run = (args, env = process.env) => execFileSync(binary, args, {
  encoding: "utf8",
  timeout: 3000,
  maxBuffer: 1024 * 1024,
  env,
}).trim();
const version = run(["--version"]);
if (version !== `infimount_mcp ${expected}`) throw new Error(`unexpected sidecar version: ${version}`);
const checksumPath = `${binary}.sha256`;
if (!fs.existsSync(checksumPath)) throw new Error("prepared sidecar checksum is missing");
const expectedChecksum = fs.readFileSync(checksumPath, "utf8").trim().split(/\s+/)[0];
const actualChecksum = crypto.createHash("sha256").update(fs.readFileSync(binary)).digest("hex");
if (actualChecksum !== expectedChecksum) throw new Error("sidecar checksum mismatch");
const packagedChecksum = path.join(path.dirname(binary), "mcp.sha256");
if (!fs.existsSync(packagedChecksum)) throw new Error("packaged checksum resource is missing");
if (fs.readFileSync(packagedChecksum, "utf8").trim().split(/\s+/)[0] !== actualChecksum) {
  throw new Error("packaged checksum resource mismatch");
}
const tempHome = fs.mkdtempSync(path.join(os.tmpdir(), "infimount-sidecar-smoke-"));
try {
  const report = JSON.parse(run(["doctor", "--json"], { ...process.env, HOME: tempHome }));
  if (report.healthy !== true) throw new Error("doctor did not report healthy");
  if (report.version !== expected) throw new Error("doctor version mismatch");
  if (!Array.isArray(report.checks)) throw new Error("doctor checks are missing");
} finally {
  fs.rmSync(tempHome, { recursive: true, force: true });
}
NODE

echo "Sidecar smoke passed: $BINARY ($EXPECTED_VERSION)"
