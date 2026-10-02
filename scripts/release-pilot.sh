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
  secret-tool \
  timeout
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

echo "===== RELEASE PILOT: CHECK HARNESS SYNTAX ====="
node --check "$ROOT_DIR/scripts/release-pilot-state.mjs"
node --check "$ROOT_DIR/scripts/release-pilot-webdriver.mjs"

echo
echo "===== RELEASE PILOT: SEED ISOLATED HOME ====="
HOME="$TMP_HOME" node "$ROOT_DIR/scripts/release-pilot-state.mjs" seed --home "$TMP_HOME"

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
  # unavailable. Reproduce a real headless login session rather than weakening
  # production startup behavior. gnome-keyring-daemon --login consumes the
  # login password but requires a subsequent --start to finish initialization.
  KEYRING_CONTROL_DIR="$HOME/.keyring"
  mkdir -p "$KEYRING_CONTROL_DIR"
  chmod 700 "$KEYRING_CONTROL_DIR"

  apply_keyring_env() {
    while IFS= read -r line; do
      case "$line" in
        GNOME_KEYRING_CONTROL=*|GNOME_KEYRING_PID=*|SSH_AUTH_SOCK=*)
          export "$line"
          ;;
      esac
    done
  }

  KEYRING_LOGIN_ENV="$(
    printf "%s" "infimount-release-pilot" |
      timeout 10s gnome-keyring-daemon \
        --daemonize \
        --login \
        --components=secrets \
        --control-directory="$KEYRING_CONTROL_DIR"
  )"
  apply_keyring_env <<<"$KEYRING_LOGIN_ENV"

  KEYRING_START_ENV="$(
    timeout 10s gnome-keyring-daemon --start --components=secrets
  )"
  apply_keyring_env <<<"$KEYRING_START_ENV"

  test -n "${GNOME_KEYRING_CONTROL:-}" || {
    echo "Release pilot failed: gnome-keyring did not provide GNOME_KEYRING_CONTROL" >&2
    exit 1
  }

  # Prove Secret Service is actually usable before launching Infimount.
  printf "%s" "release-pilot-canary" |
    timeout 10s secret-tool store --label="Infimount release pilot" \
      service infimount-release-pilot account canary

  CANARY="$(
    timeout 10s secret-tool lookup service infimount-release-pilot account canary
  )"
  test "$CANARY" = "release-pilot-canary" || {
    echo "Release pilot failed: transient Secret Service canary round-trip failed" >&2
    exit 1
  }

  timeout 10s secret-tool clear \
    service infimount-release-pilot account canary >/dev/null

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
echo "pagination_auto_continuation=yes"
echo "stale_cursor_recovery=yes"
echo "agent_task_publication_safety=yes"
echo "agent_task_scoped_sidecar_independent_gate=yes"
echo "agent_task_scoped_sidecar_tool_surface=yes"
echo "agent_task_scoped_sidecar_confinement=yes"
echo "========================================"
