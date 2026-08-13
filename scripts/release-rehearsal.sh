#!/usr/bin/env bash
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"; VERSION=0.8.0-rc.1; WORK_DIR=""; KEEP=0
while (($#)); do
  case "$1" in
    --) shift; continue ;;
    --version) VERSION="$2"; shift 2;;
    --work-dir) WORK_DIR="$2"; shift 2;;
    --keep) KEEP=1; shift;;
    *) echo "usage: $0 [--version X] [--work-dir DIR] [--keep]" >&2; exit 2;;
  esac
done
[[ "$VERSION" != *+* ]] || { echo 'build metadata is not supported' >&2; exit 1; }
if [[ -z "$WORK_DIR" ]]; then WORK_DIR="$(mktemp -d)"; else mkdir -p "$WORK_DIR"; fi
cleanup(){ rc=$?; if ((KEEP==0)); then rm -rf "$WORK_DIR"; else echo "Rehearsal evidence retained at $WORK_DIR"; fi; exit "$rc"; }; trap cleanup EXIT
mkdir -p "$WORK_DIR"/{dist,release-assets,downloaded-release-assets,signing,server}
export GITHUB_REF_NAME="v$VERSION" TAURI_SIGNING_PRIVATE_KEY=rehearsal TAURI_SIGNING_PRIVATE_KEY_PASSWORD=rehearsal
bash "$ROOT_DIR/scripts/check-signing-policy-rehearsal.sh" --matrix
bash "$ROOT_DIR/scripts/rehearse-release-assets.sh" "$WORK_DIR/release-assets" "$VERSION"
node "$ROOT_DIR/scripts/rehearse-release-api.mjs" "$WORK_DIR/release-assets" "$WORK_DIR/downloaded-release-assets"
bash "$ROOT_DIR/scripts/check-release-assets.sh" "$WORK_DIR/downloaded-release-assets"
bash "$ROOT_DIR/scripts/check-updater-assets.sh" "$WORK_DIR/downloaded-release-assets" "$VERSION"
node "$ROOT_DIR/scripts/rehearse-updater-client.mjs" "$WORK_DIR/downloaded-release-assets" "$VERSION"
node "$ROOT_DIR/scripts/rehearse-update-server.mjs" "$WORK_DIR/downloaded-release-assets" "$WORK_DIR/server/state.json" &
server_pid=$!; trap 'kill "$server_pid" 2>/dev/null || true' EXIT
for _ in {1..50}; do [[ -s "$WORK_DIR/server/state.json" ]] && break; sleep 0.1; done
port="$(jq -r .port "$WORK_DIR/server/state.json")"; curl --fail --silent "http://127.0.0.1:$port/latest.json" >/dev/null
kill "$server_pid" 2>/dev/null || true; wait "$server_pid" 2>/dev/null || true; trap cleanup EXIT
bash "$ROOT_DIR/scripts/rehearse-linux-packages.sh" "$WORK_DIR/downloaded-release-assets" "$VERSION"
node "$ROOT_DIR/scripts/check-release-rehearsal-fixtures.mjs" "$ROOT_DIR/tests/fixtures/release-rehearsal"
# Explicit mutation test: checksum validation must fail closed.
cp "$WORK_DIR/downloaded-release-assets/Infimount-amd64.deb" "$WORK_DIR/downloaded-release-assets/.original"
printf x >> "$WORK_DIR/downloaded-release-assets/Infimount-amd64.deb"
if bash "$ROOT_DIR/scripts/check-release-assets.sh" "$WORK_DIR/downloaded-release-assets" >/dev/null 2>&1; then echo 'mutation test unexpectedly passed' >&2; exit 1; fi
mv "$WORK_DIR/downloaded-release-assets/.original" "$WORK_DIR/downloaded-release-assets/Infimount-amd64.deb"
echo "Release rehearsal passed for $VERSION (non-publishing, no production secrets, no external network)."
