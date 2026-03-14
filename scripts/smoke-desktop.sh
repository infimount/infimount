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

# Build desktop binary outside runtime timeout so CI cold builds do not get killed.
cargo +stable build --manifest-path "$ROOT_DIR/apps/desktop/src-tauri/Cargo.toml"

DESKTOP_BIN="$ROOT_DIR/target/debug/infimount"
if [ ! -x "$DESKTOP_BIN" ]; then
  echo "Smoke check failed: expected desktop binary at $DESKTOP_BIN" >&2
  exit 1
fi

timeout 60s xvfb-run -a env \
  HOME="$TMP_HOME" \
  XDG_CONFIG_HOME="$TMP_HOME/.config" \
  CARGO_HOME="$CARGO_HOME_PATH" \
  RUSTUP_HOME="$RUSTUP_HOME_PATH" \
  "$DESKTOP_BIN" >/tmp/infimount-smoke.log 2>&1 || true

STORAGES_FILE="$TMP_HOME/.infimount/storages.json"

if [ ! -f "$STORAGES_FILE" ]; then
  echo "Smoke check failed: storages registry was not created at $STORAGES_FILE" >&2
  echo "Desktop run log:" >&2
  tail -n 200 /tmp/infimount-smoke.log >&2 || true
  exit 1
fi

if ! grep -Eq '"name"[[:space:]]*:[[:space:]]*"Smoke Home"' "$STORAGES_FILE"; then
  echo "Smoke check failed: migrated storage 'Smoke Home' was not found in $STORAGES_FILE" >&2
  echo "storages.json contents:" >&2
  cat "$STORAGES_FILE" >&2
  echo "Desktop run log:" >&2
  tail -n 200 /tmp/infimount-smoke.log >&2 || true
  exit 1
fi
