#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
INPUT_ASSET_DIR="${1:-}"
TMP_DIR="$(mktemp -d 2>/dev/null || mktemp -d -t infimount-install-smoke)"
SERVER_PID=""

cleanup() {
  if [[ -n "$SERVER_PID" ]]; then
    kill "$SERVER_PID" >/dev/null 2>&1 || true
  fi
  rm -rf "$TMP_DIR"
}
trap cleanup EXIT INT TERM

fail() {
  echo "install script smoke failed: $*" >&2
  exit 1
}

if [[ -n "$INPUT_ASSET_DIR" ]]; then
  ASSET_DIR="$INPUT_ASSET_DIR"
else
  ASSET_DIR="$TMP_DIR/assets"
  mkdir -p "$ASSET_DIR"
  printf 'dummy appimage\n' > "$ASSET_DIR/Infimount-x86_64.AppImage"
  printf 'dummy msi\n' > "$ASSET_DIR/Infimount.msi"
  printf 'dummy exe\n' > "$ASSET_DIR/Infimount-setup.exe"
  (
    cd "$ASSET_DIR"
    sha256sum Infimount-x86_64.AppImage Infimount.msi Infimount-setup.exe > SHA256SUMS.txt
  )
fi

for asset in Infimount-x86_64.AppImage Infimount.msi Infimount-setup.exe SHA256SUMS.txt; do
  [[ -s "$ASSET_DIR/$asset" ]] || fail "$asset is missing from $ASSET_DIR"
done

PORT_FILE="$TMP_DIR/port"
python3 - <<'PY' "$ASSET_DIR" "$PORT_FILE" &
import functools
import http.server
import pathlib
import socketserver
import sys

asset_dir = pathlib.Path(sys.argv[1]).resolve()
port_file = pathlib.Path(sys.argv[2])
handler = functools.partial(http.server.SimpleHTTPRequestHandler, directory=str(asset_dir))
with socketserver.TCPServer(("127.0.0.1", 0), handler) as httpd:
    port_file.write_text(str(httpd.server_address[1]))
    httpd.serve_forever()
PY
SERVER_PID=$!

for _ in {1..50}; do
  [[ -s "$PORT_FILE" ]] && break
  sleep 0.1
done
[[ -s "$PORT_FILE" ]] || fail "local asset server did not start"
BASE_URL="http://127.0.0.1:$(cat "$PORT_FILE")"

INFIMOUNT_RELEASE_BASE_URL="$BASE_URL" \
INFIMOUNT_INSTALL_FORMAT=appimage \
INFIMOUNT_INSTALL_DRY_RUN=1 \
"$ROOT_DIR/scripts/install.sh"

if command -v pwsh >/dev/null 2>&1; then
  INFIMOUNT_RELEASE_BASE_URL="$BASE_URL" \
  INFIMOUNT_INSTALL_DRY_RUN=1 \
  pwsh -NoProfile -ExecutionPolicy Bypass -File "$ROOT_DIR/scripts/install.ps1" -Installer msi

  INFIMOUNT_RELEASE_BASE_URL="$BASE_URL" \
  INFIMOUNT_INSTALL_DRY_RUN=1 \
  pwsh -NoProfile -ExecutionPolicy Bypass -File "$ROOT_DIR/scripts/install.ps1" -Installer exe
else
  echo "pwsh not found; skipping PowerShell install script smoke."
fi

printf 'Install script smoke passed.\n'
