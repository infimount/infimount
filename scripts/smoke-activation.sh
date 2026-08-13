#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

node scripts/prepare-mcp-sidecar.mjs
TARGET="${TARGET:-$(rustc --print host-tuple)}"
EXT=""
[[ "$TARGET" == *windows* ]] && EXT=".exe"
SIDECAR="$ROOT/apps/desktop/src-tauri/binaries/mcp-${TARGET}${EXT}"

bash scripts/smoke-mcp-sidecar.sh "$SIDECAR"
[[ "$($SIDECAR print-config-dir)" == /* || "$TARGET" == *windows* ]] || {
  echo "print-config-dir did not return an absolute path" >&2
  exit 1
}
if "$SIDECAR" definitely-not-a-command >/dev/null 2>&1; then
  echo "sidecar unexpectedly accepted an unknown command" >&2
  exit 1
fi

INFIMOUNT_MCP_PATH="$SIDECAR" cargo test -p infimount \
  complete_demo_activation_over_packaged_stdio_sidecar -- --ignored
