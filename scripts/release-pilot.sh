#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
ORIGINAL_HOME="${HOME}"
CARGO_HOME_PATH="${CARGO_HOME:-$ORIGINAL_HOME/.cargo}"
RUSTUP_HOME_PATH="${RUSTUP_HOME:-$ORIGINAL_HOME/.rustup}"
TMP_HOME="${INFIMOUNT_PILOT_HOME:-$(mktemp -d)}"
OWNS_TMP_HOME=0

if [[ -z "${INFIMOUNT_PILOT_HOME:-}" ]]; then
  OWNS_TMP_HOME=1
fi

cleanup() {
  if [[ "$OWNS_TMP_HOME" == "1" && "${INFIMOUNT_PILOT_KEEP_HOME:-0}" != "1" ]]; then
    rm -rf "$TMP_HOME"
  elif [[ "${INFIMOUNT_PILOT_KEEP_HOME:-0}" == "1" ]]; then
    echo "Release pilot HOME preserved at $TMP_HOME"
  fi
}
trap cleanup EXIT

require_command() {
  command -v "$1" >/dev/null 2>&1 || {
    echo "Release pilot failed: required command '$1' is unavailable" >&2
    exit 1
  }
}

if [[ "$(uname -s)" != "Linux" ]]; then
  echo "Release pilot failed: direct tauri-driver automation currently runs on Linux only" >&2
  exit 1
fi

for command in \
  node \
  pnpm \
  cargo \
  rustc \
  tauri-driver \
  WebKitWebDriver \
  xvfb-run \
  dbus-run-session \
  gnome-keyring-daemon \
  secret-tool
do
  require_command "$command"
done

if [[ -e "$TMP_HOME/.infimount" || -e "$TMP_HOME/.release-pilot-baseline" ]]; then
  echo "Release pilot failed: pilot HOME already contains release-pilot state: $TMP_HOME" >&2
  exit 1
fi

mkdir -p "$TMP_HOME"
chmod 700 "$TMP_HOME"
mkdir -p "$TMP_HOME/.config" "$TMP_HOME/.runtime"
chmod 700 "$TMP_HOME/.runtime"

echo "===== RELEASE PILOT: SEED ISOLATED HOME ====="
HOME="$TMP_HOME" node "$ROOT_DIR/scripts/release-pilot-state.mjs" seed --home "$TMP_HOME"

PILOT_KIT="$TMP_HOME/pilot-kit"
HOME="$TMP_HOME" node "$ROOT_DIR/scripts/agent-task-pilot-kit-core.mjs" prepare --out "$PILOT_KIT"   >"$TMP_HOME/pilot-kit-summary.json"
export INFIMOUNT_PILOT_KIT="$PILOT_KIT"

echo
echo "===== RELEASE PILOT: BUILD REAL DESKTOP + SIDECAR ====="
pnpm --dir "$ROOT_DIR/apps/desktop" build

# The Tauri CLI owns the production-vs-development build context. A plain
# cargo build still honors devUrl, even in the release profile. --no-bundle
# embeds frontendDist like a release build while skipping installer generation.
CARGO_HOME="$CARGO_HOME_PATH" \
RUSTUP_HOME="$RUSTUP_HOME_PATH" \
  pnpm --dir "$ROOT_DIR/apps/desktop" tauri build --no-bundle

TARGET_TRIPLE="$(rustc --print host-tuple)"
PREPARED_SIDECAR="$ROOT_DIR/apps/desktop/src-tauri/binaries/mcp-$TARGET_TRIPLE"
DESKTOP_BIN="$ROOT_DIR/target/release/infimount"
RUNTIME_SIDECAR="$ROOT_DIR/target/release/mcp"
RUNTIME_CHECKSUM_DIR="$ROOT_DIR/target/release/binaries"
RUNTIME_CHECKSUM="$RUNTIME_CHECKSUM_DIR/mcp.sha256"

test -x "$DESKTOP_BIN" || {
  echo "Release pilot failed: desktop binary missing at $DESKTOP_BIN" >&2
  exit 1
}

test -x "$PREPARED_SIDECAR" || {
  echo "Release pilot failed: prepared sidecar missing at $PREPARED_SIDECAR" >&2
  exit 1
}

mkdir -p "$RUNTIME_CHECKSUM_DIR"
cp "$PREPARED_SIDECAR" "$RUNTIME_SIDECAR"
chmod 755 "$RUNTIME_SIDECAR"
cp "$ROOT_DIR/apps/desktop/src-tauri/binaries/mcp.sha256" "$RUNTIME_CHECKSUM"

"$RUNTIME_SIDECAR" --version

echo
echo "===== RELEASE PILOT: REAL TAURI WEBDRIVER ====="

# DISPLAY must exist before the D-Bus session starts. GTK portal services are
# D-Bus activated by WebKit/Tauri and inherit the D-Bus daemon environment.
# Starting Xvfb inside dbus-run-session leaves those helpers without DISPLAY.
xvfb-run -a dbus-run-session -- bash -c '
  set -euo pipefail

  export HOME="$1"
  export XDG_CONFIG_HOME="$1/.config"
  export XDG_RUNTIME_DIR="$1/.runtime"
  export CARGO_HOME="$2"
  export RUSTUP_HOME="$3"
  export INFIMOUNT_PILOT_HOME="$1"
  export INFIMOUNT_PILOT_APP="$4"
  export INFIMOUNT_PILOT_SIDECAR="$5"
  export WEBKIT_DISABLE_COMPOSITING_MODE=1
  export GDK_BACKEND=x11

  # AppState intentionally fails closed when native Secret Service is
  # unavailable. Use a transient real Secret Service instead of weakening
  # production startup behavior for CI.
  printf "%s\n" "infimount-release-pilot" |
    gnome-keyring-daemon --unlock --components=secrets >/dev/null

  # Prove Secret Service is actually usable before launching Infimount.
  printf "%s" "release-pilot-canary" |
    secret-tool store --label="Infimount release pilot" \
      service infimount-release-pilot account canary

  test "$(secret-tool lookup service infimount-release-pilot account canary)" = "release-pilot-canary" || {
    echo "Release pilot failed: transient Secret Service canary round-trip failed" >&2
    exit 1
  }

  secret-tool clear service infimount-release-pilot account canary >/dev/null

  node "$6"
' bash \
  "$TMP_HOME" \
  "$CARGO_HOME_PATH" \
  "$RUSTUP_HOME_PATH" \
  "$DESKTOP_BIN" \
  "$RUNTIME_SIDECAR" \
  "$ROOT_DIR/scripts/release-pilot-webdriver.mjs"

echo
echo "========================================"
echo "RELEASE PILOT PASSED"
echo "storage_only_onboarding=yes"
echo "legacy_home_normalization=yes"
echo "read_only_agent_access=yes"
echo "read_write_agent_access=yes"
echo "packaged_sidecar_safety_probe=yes"
echo "stdio_no_background_start=yes"
echo "http_explicit_start_stop=yes"
echo "ordinary_stdio_disabled_gate=yes"
echo "guided_disabled_setup_returns_to_stdio=yes"
echo "pagination_450_entries=yes"
echo "pagination_stale_cursor_recovery=yes"
echo "agent_task_coding=yes"
echo "agent_task_document=yes"
echo "agent_task_data_analysis=yes"
echo "agent_task_publication_safety=yes"
echo "========================================"
