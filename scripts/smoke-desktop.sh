#!/usr/bin/env bash

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TMP_HOME="$(mktemp -d)"
ORIGINAL_HOME="${HOME}"
CARGO_HOME_PATH="${CARGO_HOME:-$ORIGINAL_HOME/.cargo}"
RUSTUP_HOME_PATH="${RUSTUP_HOME:-$ORIGINAL_HOME/.rustup}"
cleanup() {
  rm -rf "$TMP_HOME"
}
trap cleanup EXIT

mkdir -p "$TMP_HOME/.infimount"
cat > "$TMP_HOME/.infimount/config.json" <<'EOF'
[
  {
    "id": "legacy-local",
    "name": "Smoke Home",
    "kind": "local",
    "root": "/tmp",
    "config": {}
  }
]
EOF

pnpm --dir "$ROOT_DIR/apps/desktop" build

timeout 30s xvfb-run -a env \
  HOME="$TMP_HOME" \
  XDG_CONFIG_HOME="$TMP_HOME/.config" \
  CARGO_HOME="$CARGO_HOME_PATH" \
  RUSTUP_HOME="$RUSTUP_HOME_PATH" \
  cargo +stable run --manifest-path "$ROOT_DIR/apps/desktop/src-tauri/Cargo.toml" >/tmp/infimount-smoke.log 2>&1 || true

test -f "$TMP_HOME/.infimount/storages.json"
grep -q '"name": "Smoke Home"' "$TMP_HOME/.infimount/storages.json"
