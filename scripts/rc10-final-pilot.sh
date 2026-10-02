#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
ORIGINAL_HOME="$HOME"
ORIGINAL_PATH="$PATH"
PILOT_ROOT="${INFIMOUNT_RC10_PILOT_ROOT:-$ORIGINAL_HOME/.infimount-rc10-final-pilot}"
PILOT_HOME="$PILOT_ROOT/home"
KIT="$PILOT_HOME/pilot-kit"
EVIDENCE="$PILOT_ROOT/evidence"
CODEX_HOME_PATH="${CODEX_HOME:-$ORIGINAL_HOME/.codex}"
CANDIDATE_VERSION="0.8.1-rc.10"
CANDIDATE_COMMIT="86859130f93757d6893bc128e3955906a4f0130e"

RESET=0
for arg in "$@"; do
  case "$arg" in
    --reset) RESET=1 ;;
    *)
      echo "usage: $0 [--reset]" >&2
      exit 2
      ;;
  esac
done

if [[ "$(uname -s)" != "Linux" || "$(dpkg --print-architecture)" != "amd64" ]]; then
  echo "RC10 real pilot currently targets Linux amd64." >&2
  exit 1
fi

if [[ "$RESET" == "1" && -e "$PILOT_ROOT" ]]; then
  case "$PILOT_ROOT" in
    "$ORIGINAL_HOME"/.infimount-rc10-final-pilot|"$ORIGINAL_HOME"/.infimount-rc10-final-pilot/*)
      rm -rf "$PILOT_ROOT"
      ;;
    *)
      echo "Refusing --reset for unexpected pilot root: $PILOT_ROOT" >&2
      exit 1
      ;;
  esac
fi

if [[ -e "$PILOT_ROOT" ]]; then
  echo "RC10 real pilot root already exists: $PILOT_ROOT" >&2
  echo "Preserve any evidence you need, then rerun with --reset to restart the isolated pilot." >&2
  exit 1
fi

for command in node pnpm cargo git curl sudo apt-get dpkg-query; do
  command -v "$command" >/dev/null 2>&1 || {
    echo "RC10 real pilot failed: required command '$command' is unavailable." >&2
    exit 1
  }
done

CODEX_BIN="$(command -v codex || true)"
if [[ -z "$CODEX_BIN" || ! -x "$CODEX_BIN" ]]; then
  echo "RC10 real pilot failed: Codex CLI is not installed or not in PATH." >&2
  exit 1
fi
"$CODEX_BIN" --version

if [[ ! -d "$CODEX_HOME_PATH" ]]; then
  echo "RC10 real pilot failed: Codex home does not exist: $CODEX_HOME_PATH" >&2
  echo "Use your authenticated Codex CLI environment or set CODEX_HOME explicitly." >&2
  exit 1
fi

echo "===== RC10 FINAL PILOT: STABLE INSTALLER UPGRADE ====="
INFIMOUNT_RC10_PILOT_ROOT="$PILOT_ROOT"   bash "$ROOT_DIR/scripts/rc10-upgrade-pilot.sh"

test "$(dpkg-query -W -f='${Version}' infimount)" = "$CANDIDATE_VERSION"
APP_BINARY="$(readlink -f "$(command -v infimount)")"
test -x "$APP_BINARY"

echo
echo "===== RC10 FINAL PILOT: PREPARE CANONICAL WORKLOADS ====="
node "$ROOT_DIR/scripts/agent-task-pilot-kit.mjs" prepare   --candidate-version "$CANDIDATE_VERSION"   --candidate-commit "$CANDIDATE_COMMIT"   --out "$KIT"

mkdir -p "$KIT/coding/publish-destination" "$KIT/data/publish-destination"
mkdir -p "$PILOT_HOME/.config" "$PILOT_HOME/.runtime"
chmod 700 "$PILOT_HOME/.config" "$PILOT_HOME/.runtime"

# The Infimount handoff launches a login bash inside a terminal. Keep HOME
# isolated for Infimount while preserving the user's Codex executable path and
# authenticated CODEX_HOME.
printf 'export PATH=%q\nexport CODEX_HOME=%q\n'   "$ORIGINAL_PATH" "$CODEX_HOME_PATH" >"$PILOT_HOME/.profile"
chmod 600 "$PILOT_HOME/.profile"

echo
echo "===== RC10 FINAL PILOT: ENSURE DESKTOP AUTOMATION DEPENDENCIES ====="
missing_apt=()
command -v WebKitWebDriver >/dev/null 2>&1 || missing_apt+=(webkit2gtk-driver)
command -v xvfb-run >/dev/null 2>&1 || missing_apt+=(xvfb)
command -v dbus-run-session >/dev/null 2>&1 || missing_apt+=(dbus-x11)
command -v gnome-keyring-daemon >/dev/null 2>&1 || missing_apt+=(gnome-keyring)
command -v secret-tool >/dev/null 2>&1 || missing_apt+=(libsecret-tools)
command -v x-terminal-emulator >/dev/null 2>&1 || missing_apt+=(xterm)

if [[ "${#missing_apt[@]}" -gt 0 ]]; then
  sudo apt-get update
  sudo apt-get install -y "${missing_apt[@]}"
fi

if ! command -v tauri-driver >/dev/null 2>&1; then
  cargo install tauri-driver --version 2.1.0 --locked
fi

for command in WebKitWebDriver xvfb-run dbus-run-session gnome-keyring-daemon secret-tool x-terminal-emulator tauri-driver timeout; do
  command -v "$command" >/dev/null 2>&1 || {
    echo "RC10 real pilot failed: dependency '$command' is still unavailable." >&2
    exit 1
  }
done

echo
echo "===== RC10 FINAL PILOT: THREE REAL CODEX WORKLOADS ====="

xvfb-run -a dbus-run-session -- bash -c '
  set -euo pipefail

  export HOME="$1"
  export XDG_CONFIG_HOME="$1/.config"
  export XDG_RUNTIME_DIR="$1/.runtime"
  export INFIMOUNT_RC10_PILOT_HOME="$1"
  export INFIMOUNT_RC10_PILOT_ROOT="$2"
  export INFIMOUNT_RC10_PILOT_KIT="$3"
  export INFIMOUNT_RC10_PILOT_EVIDENCE="$4"
  export INFIMOUNT_RC10_PILOT_APP="$5"
  export CODEX_HOME="$6"
  export PATH="$7"
  export SHELL=/bin/bash
  export WEBKIT_DISABLE_COMPOSITING_MODE=1
  export GDK_BACKEND=x11

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
    printf "%s" "infimount-rc10-real-pilot" |
      timeout 10s gnome-keyring-daemon         --daemonize         --login         --components=secrets         --control-directory="$KEYRING_CONTROL_DIR"
  )"
  apply_keyring_env <<<"$KEYRING_LOGIN_ENV"

  KEYRING_START_ENV="$(
    timeout 10s gnome-keyring-daemon --start --components=secrets
  )"
  apply_keyring_env <<<"$KEYRING_START_ENV"

  test -n "${GNOME_KEYRING_CONTROL:-}" || {
    echo "RC10 real pilot failed: transient keyring did not initialize." >&2
    exit 1
  }

  printf "%s" "rc10-real-pilot-canary" |
    timeout 10s secret-tool store --label="Infimount rc10 real pilot"       service infimount-rc10-real-pilot account canary

  test "$(
    timeout 10s secret-tool lookup service infimount-rc10-real-pilot account canary
  )" = "rc10-real-pilot-canary"

  timeout 10s secret-tool clear     service infimount-rc10-real-pilot account canary >/dev/null

  node "$8"
' bash   "$PILOT_HOME"   "$PILOT_ROOT"   "$KIT"   "$EVIDENCE"   "$APP_BINARY"   "$CODEX_HOME_PATH"   "$ORIGINAL_PATH"   "$ROOT_DIR/scripts/rc10-real-agent-pilot.mjs"

echo
echo "========================================"
echo "RC10 AUTOMATED REAL PILOT COMPLETE"
echo "candidate=$CANDIDATE_VERSION"
echo "candidate_commit=$CANDIDATE_COMMIT"
echo "upgrade=v0.8.0->rc.10 passed"
echo "real_codex_workloads=coding,document,data-analysis"
echo "deterministic_output_checks=passed"
echo "safety_probes=passed"
echo "quality_acceptance=pending"
echo "private_pilot_root=$PILOT_ROOT"
echo
echo "Review:"
echo "  $EVIDENCE/QUALITY-REVIEW.md"
echo
echo "After genuinely reviewing the outputs, accept with:"
echo "  node scripts/rc10-final-pilot-accept.mjs --root '$PILOT_ROOT' --accept-quality"
echo "========================================"
