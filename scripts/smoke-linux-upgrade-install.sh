#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CANDIDATE_DEB="${1:-}"
CANDIDATE_VERSION="${2:-}"
REPO="infimount/infimount"

fail() {
  echo "Linux installer upgrade pilot failed: $*" >&2
  exit 1
}

if [[ -z "$CANDIDATE_DEB" || -z "$CANDIDATE_VERSION" ]]; then
  echo "usage: $0 <candidate-deb> <candidate-version>" >&2
  exit 2
fi

[[ "$(uname -s)" == "Linux" ]] || fail "Linux is required"
[[ "$(uname -m)" == "x86_64" ]] || fail "the published amd64 upgrade pilot requires x86_64"

for command in \
  gh \
  jq \
  dpkg \
  dpkg-deb \
  sha256sum \
  python3 \
  sudo \
  apt-get \
  xvfb-run \
  dbus-run-session \
  gnome-keyring-daemon \
  secret-tool \
  timeout
do
  command -v "$command" >/dev/null 2>&1 || fail "required command '$command' is unavailable"
done

[[ -n "${GH_TOKEN:-}" ]] || fail "GH_TOKEN is required to resolve the previous stable release"
[[ -f "$CANDIDATE_DEB" ]] || fail "candidate .deb is missing: $CANDIDATE_DEB"
[[ "$CANDIDATE_VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?$ ]] ||
  fail "candidate version is not SemVer-like: $CANDIDATE_VERSION"

WORK_DIR="$(mktemp -d)"
PILOT_HOME="$WORK_DIR/home"
STABLE_DIR="$WORK_DIR/stable"
CANDIDATE_COPY="$WORK_DIR/Infimount-candidate-amd64.deb"
INSTALLED_BY_TEST=0
APP_PID=""

cleanup() {
  local rc=$?
  trap - EXIT
  if [[ -n "$APP_PID" ]]; then
    kill "$APP_PID" >/dev/null 2>&1 || true
    wait "$APP_PID" >/dev/null 2>&1 || true
  fi
  if [[ "$INSTALLED_BY_TEST" == "1" ]]; then
    sudo apt-get remove -y infimount >/dev/null 2>&1 || true
  fi
  rm -rf "$WORK_DIR"
  exit "$rc"
}
trap cleanup EXIT

mkdir -p "$STABLE_DIR" "$PILOT_HOME/.infimount" "$PILOT_HOME/stable-storage"
chmod 755 "$WORK_DIR" "$STABLE_DIR"
chmod 700 "$PILOT_HOME" "$PILOT_HOME/.infimount"
cp "$CANDIDATE_DEB" "$CANDIDATE_COPY"
chmod 644 "$CANDIDATE_COPY"

CANDIDATE_TAG="v$CANDIDATE_VERSION"
BASE_TAG=""
mapfile -t STABLE_TAGS < <(
  gh api "repos/$REPO/releases?per_page=100" \
    --jq '.[] | select(.draft == false and .prerelease == false) | .tag_name'
)
for tag in "${STABLE_TAGS[@]}"; do
  if [[ "$tag" != "$CANDIDATE_TAG" ]]; then
    BASE_TAG="$tag"
    break
  fi
done
[[ -n "$BASE_TAG" ]] || fail "could not resolve a previous stable release"

echo "candidate_tag=$CANDIDATE_TAG"
echo "upgrade_base_tag=$BASE_TAG"

gh release download "$BASE_TAG" \
  --repo "$REPO" \
  --pattern "Infimount-amd64.deb" \
  --pattern "SHA256SUMS.txt" \
  --dir "$STABLE_DIR"

STABLE_DEB="$STABLE_DIR/Infimount-amd64.deb"
STABLE_SUMS="$STABLE_DIR/SHA256SUMS.txt"
[[ -s "$STABLE_DEB" ]] || fail "stable .deb was not downloaded"
[[ -s "$STABLE_SUMS" ]] || fail "stable SHA256SUMS.txt was not downloaded"

(
  cd "$STABLE_DIR"
  grep -E '(^|[[:space:]])Infimount-amd64\.deb$' SHA256SUMS.txt > selected.sha256
  [[ "$(wc -l < selected.sha256)" -eq 1 ]] ||
    fail "stable checksums must contain exactly one Infimount-amd64.deb entry"
  sha256sum -c selected.sha256
)

STABLE_PACKAGE="$(dpkg-deb -f "$STABLE_DEB" Package)"
CANDIDATE_PACKAGE="$(dpkg-deb -f "$CANDIDATE_COPY" Package)"
STABLE_VERSION="$(dpkg-deb -f "$STABLE_DEB" Version)"
DEB_CANDIDATE_VERSION="$(dpkg-deb -f "$CANDIDATE_COPY" Version)"
STABLE_ARCH="$(dpkg-deb -f "$STABLE_DEB" Architecture)"
CANDIDATE_ARCH="$(dpkg-deb -f "$CANDIDATE_COPY" Architecture)"

[[ "$STABLE_PACKAGE" == "infimount" ]] || fail "unexpected stable package: $STABLE_PACKAGE"
[[ "$CANDIDATE_PACKAGE" == "$STABLE_PACKAGE" ]] || fail "candidate package name differs from stable package"
[[ "$STABLE_ARCH" == "amd64" && "$CANDIDATE_ARCH" == "amd64" ]] ||
  fail "both packages must be amd64"
[[ "$DEB_CANDIDATE_VERSION" == "$CANDIDATE_VERSION" ]] ||
  fail "candidate .deb version '$DEB_CANDIDATE_VERSION' does not match '$CANDIDATE_VERSION'"
dpkg --compare-versions "$STABLE_VERSION" lt "$CANDIDATE_VERSION" ||
  fail "upgrade base $STABLE_VERSION is not older than candidate $CANDIDATE_VERSION"

if dpkg-query -W -f='${Status}' "$STABLE_PACKAGE" 2>/dev/null | grep -Fq 'install ok installed'; then
  fail "runner already has infimount installed; refusing to mutate an unknown installation"
fi

cat >"$PILOT_HOME/.infimount/config.json" <<EOF
[
  {
    "id": "upgrade-pilot-local",
    "name": "Upgrade Pilot Local",
    "kind": "local",
    "root": "$PILOT_HOME/stable-storage",
    "config": {}
  }
]
EOF

run_packaged_app() {
  local phase=$1
  local binary=$2
  local require_registry=$3

  xvfb-run -a dbus-run-session -- bash -c '
    set -euo pipefail

    phase="$1"
    app="$2"
    home="$3"
    require_registry="$4"

    export HOME="$home"
    export XDG_CONFIG_HOME="$HOME/.config"
    export XDG_DATA_HOME="$HOME/.local/share"
    export XDG_RUNTIME_DIR="$HOME/.runtime"
    export WEBKIT_DISABLE_COMPOSITING_MODE=1
    export GDK_BACKEND=x11

    mkdir -p "$XDG_CONFIG_HOME" "$XDG_DATA_HOME/keyrings" "$XDG_RUNTIME_DIR"
    chmod 700 "$HOME" "$XDG_RUNTIME_DIR"

    KEYRING_CONTROL_DIR="$HOME/.keyring"
    rm -rf "$KEYRING_CONTROL_DIR"
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
      printf "%s" "infimount-upgrade-pilot" |
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
      echo "Upgrade pilot failed: keyring control environment is missing" >&2
      exit 1
    }

    printf "%s" "upgrade-pilot-canary" |
      timeout 10s secret-tool store --label="Infimount upgrade pilot" \
        service infimount-upgrade-pilot account canary
    test "$(
      timeout 10s secret-tool lookup service infimount-upgrade-pilot account canary
    )" = "upgrade-pilot-canary" || {
      echo "Upgrade pilot failed: Secret Service canary round-trip failed" >&2
      exit 1
    }
    timeout 10s secret-tool clear \
      service infimount-upgrade-pilot account canary >/dev/null

    log="$HOME/${phase}.log"
    "$app" >"$log" 2>&1 &
    pid=$!
    cleanup_app() {
      kill "$pid" >/dev/null 2>&1 || true
      wait "$pid" >/dev/null 2>&1 || true
    }
    trap cleanup_app EXIT

    ready=0
    for attempt in $(seq 1 100); do
      if ! kill -0 "$pid" >/dev/null 2>&1; then
        wait "$pid" || rc=$?
        echo "Upgrade pilot failed: $phase app exited early with ${rc:-0}" >&2
        tail -n 200 "$log" >&2 || true
        exit 1
      fi
      if [[ "$require_registry" == "yes" ]]; then
        if [[ -s "$HOME/.infimount/storages.json" ]]; then
          ready=1
          break
        fi
      elif (( attempt >= 15 )); then
        ready=1
        break
      fi
      sleep 0.1
    done

    [[ "$ready" == "1" ]] || {
      echo "Upgrade pilot failed: $phase app did not become ready" >&2
      tail -n 200 "$log" >&2 || true
      exit 1
    }

    sleep 0.3
  ' bash "$phase" "$binary" "$PILOT_HOME" "$require_registry"
}

state_manifest() {
  local target=$1
  python3 - "$PILOT_HOME/.infimount" "$target" <<'PY'
import hashlib
import json
import pathlib
import sys

root = pathlib.Path(sys.argv[1])
out = pathlib.Path(sys.argv[2])
manifest = {}
for file in sorted(path for path in root.rglob("*") if path.is_file()):
    rel = file.relative_to(root).as_posix()
    manifest[rel] = hashlib.sha256(file.read_bytes()).hexdigest()
out.write_text(json.dumps(manifest, sort_keys=True, indent=2) + "\n")
PY
}

echo "===== INSTALL PREVIOUS STABLE ====="
sudo apt-get install -y "$STABLE_DEB"
INSTALLED_BY_TEST=1
INSTALLED_STABLE="$(dpkg-query -W -f='${Version}' "$STABLE_PACKAGE")"
[[ "$INSTALLED_STABLE" == "$STABLE_VERSION" ]] ||
  fail "installed stable version '$INSTALLED_STABLE' does not match package '$STABLE_VERSION'"

hash -r
STABLE_BIN="$(command -v infimount || true)"
[[ -x "$STABLE_BIN" ]] || fail "stable desktop binary is not installed"

STABLE_SIDECAR=""
while IFS= read -r file; do
  if [[ -f "$file" && -x "$file" && "$(basename "$file")" == "mcp" ]]; then
    STABLE_SIDECAR="$file"
    break
  fi
done < <(dpkg -L "$STABLE_PACKAGE")
[[ -n "$STABLE_SIDECAR" && -x "$STABLE_SIDECAR" ]] ||
  fail "stable packaged MCP sidecar was not found"
[[ "$("$STABLE_SIDECAR" --version)" == "infimount_mcp $STABLE_VERSION" ]] ||
  fail "stable packaged sidecar version mismatch"

run_packaged_app stable "$STABLE_BIN" yes

STORAGES="$PILOT_HOME/.infimount/storages.json"
[[ -s "$STORAGES" ]] || fail "stable app did not create storages.json"

python3 - "$STORAGES" "$WORK_DIR/stable-storage-summary.json" <<'PY'
import json
import pathlib
import sys

storages = json.loads(pathlib.Path(sys.argv[1]).read_text())
if not isinstance(storages, list) or len(storages) != 1:
    raise SystemExit("stable app did not persist exactly one storage")
storage = storages[0]
if storage.get("name") != "Upgrade Pilot Local":
    raise SystemExit("stable storage identity was not preserved")
summary = {
    "id": storage.get("id"),
    "name": storage.get("name"),
    "backend": storage.get("backend"),
    "enabled": storage.get("enabled"),
    "mcp_exposed": storage.get("mcp_exposed"),
}
pathlib.Path(sys.argv[2]).write_text(json.dumps(summary, sort_keys=True) + "\n")
PY

state_manifest "$WORK_DIR/before-candidate-install.json"

echo "===== INSTALL CANDIDATE OVER STABLE ====="
sudo apt-get install -y "$CANDIDATE_COPY"
INSTALLED_CANDIDATE="$(dpkg-query -W -f='${Version}' "$CANDIDATE_PACKAGE")"
[[ "$INSTALLED_CANDIDATE" == "$CANDIDATE_VERSION" ]] ||
  fail "installed candidate version '$INSTALLED_CANDIDATE' does not match '$CANDIDATE_VERSION'"

state_manifest "$WORK_DIR/after-candidate-install.json"
cmp -s "$WORK_DIR/before-candidate-install.json" "$WORK_DIR/after-candidate-install.json" ||
  fail "candidate package installation changed user configuration before first launch"

hash -r
CANDIDATE_BIN="$(command -v infimount || true)"
[[ -x "$CANDIDATE_BIN" ]] || fail "candidate desktop binary is not installed"

CANDIDATE_SIDECAR=""
while IFS= read -r file; do
  if [[ -f "$file" && -x "$file" && "$(basename "$file")" == "mcp" ]]; then
    CANDIDATE_SIDECAR="$file"
    break
  fi
done < <(dpkg -L "$CANDIDATE_PACKAGE")
[[ -n "$CANDIDATE_SIDECAR" && -x "$CANDIDATE_SIDECAR" ]] ||
  fail "candidate packaged MCP sidecar was not found"
[[ "$("$CANDIDATE_SIDECAR" --version)" == "infimount_mcp $CANDIDATE_VERSION" ]] ||
  fail "candidate packaged sidecar version mismatch"

run_packaged_app candidate "$CANDIDATE_BIN" no

python3 - "$STORAGES" "$WORK_DIR/stable-storage-summary.json" <<'PY'
import json
import pathlib
import sys

storages = json.loads(pathlib.Path(sys.argv[1]).read_text())
before = json.loads(pathlib.Path(sys.argv[2]).read_text())
if not isinstance(storages, list) or len(storages) != 1:
    raise SystemExit("candidate launch changed the stable storage count")
storage = storages[0]
after = {
    "id": storage.get("id"),
    "name": storage.get("name"),
    "backend": storage.get("backend"),
    "enabled": storage.get("enabled"),
    "mcp_exposed": storage.get("mcp_exposed"),
}
if after != before:
    raise SystemExit(
        "candidate launch did not retain the stable storage identity/exposure state"
    )
PY

echo "=============================================="
echo "LINUX INSTALLER UPGRADE PILOT PASSED"
echo "base_tag=$BASE_TAG"
echo "from_version=$STABLE_VERSION"
echo "to_version=$CANDIDATE_VERSION"
echo "stable_package_launch=yes"
echo "installer_touched_user_state=no"
echo "candidate_package_launch=yes"
echo "storage_state_retained=yes"
echo "packaged_sidecar_version_match=yes"
echo "=============================================="
