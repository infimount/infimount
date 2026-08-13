#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ARGS=()
if [[ -n "${TARGET:-}" ]]; then
  ARGS+=(--target "$TARGET")
fi
exec node "$SCRIPT_DIR/prepare-mcp-sidecar.mjs" "${ARGS[@]}"
