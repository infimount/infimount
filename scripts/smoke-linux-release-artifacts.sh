#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
ARTIFACT_DIR="${1:-$ROOT_DIR/out/linux}"
APPIMAGE="$ARTIFACT_DIR/Infimount-x86_64.AppImage"
DEB="$ARTIFACT_DIR/Infimount-amd64.deb"
RPM="$ARTIFACT_DIR/Infimount-x86_64.rpm"

require_file() {
  if [ ! -s "$1" ]; then
    echo "Linux artifact smoke failed: missing or empty artifact: $1" >&2
    exit 1
  fi
}

make_smoke_home() {
  local tmp_home=$1
  mkdir -p "$tmp_home/.infimount"
  cat > "$tmp_home/.infimount/config.json" <<'JSON'
[
  {
    "id": "legacy-local",
    "name": "Linux Artifact Smoke Home",
    "kind": "local",
    "root": "/tmp",
    "config": {}
  }
]
JSON
}

assert_migrated_storage() {
  local tmp_home=$1
  local log_file=$2
  local storages_file="$tmp_home/.infimount/storages.json"

  if [ ! -f "$storages_file" ]; then
    echo "Linux artifact smoke failed: storages registry was not created at $storages_file" >&2
    echo "Desktop run log:" >&2
    tail -n 200 "$log_file" >&2 || true
    exit 1
  fi

  if ! grep -Eq '"name"[[:space:]]*:[[:space:]]*"Linux Artifact Smoke Home"' "$storages_file"; then
    echo "Linux artifact smoke failed: migrated storage was not found in $storages_file" >&2
    echo "storages.json contents:" >&2
    cat "$storages_file" >&2
    echo "Desktop run log:" >&2
    tail -n 200 "$log_file" >&2 || true
    exit 1
  fi
}

run_desktop_for_migration() {
  local command_path=$1
  local tmp_home
  local log_file
  tmp_home="$(mktemp -d)"
  log_file="$(mktemp)"
  make_smoke_home "$tmp_home"

  timeout 60s xvfb-run -a env \
    HOME="$tmp_home" \
    XDG_CONFIG_HOME="$tmp_home/.config" \
    APPIMAGE_EXTRACT_AND_RUN=1 \
    "$command_path" >"$log_file" 2>&1 || true

  assert_migrated_storage "$tmp_home" "$log_file"
  rm -rf "$tmp_home" "$log_file"
}

require_file "$APPIMAGE"
require_file "$DEB"
require_file "$RPM"

if ! command -v xvfb-run >/dev/null 2>&1; then
  echo "Linux artifact smoke failed: xvfb-run is required" >&2
  exit 1
fi

if ! command -v dpkg-deb >/dev/null 2>&1; then
  echo "Linux artifact smoke failed: dpkg-deb is required" >&2
  exit 1
fi

if ! command -v rpm >/dev/null 2>&1; then
  echo "Linux artifact smoke failed: rpm is required" >&2
  exit 1
fi

chmod +x "$APPIMAGE"
"$APPIMAGE" --appimage-version >/dev/null
run_desktop_for_migration "$APPIMAGE"

dpkg-deb --info "$DEB" >/dev/null
DEB_PACKAGE="$(dpkg-deb --field "$DEB" Package)"
if [ -z "$DEB_PACKAGE" ]; then
  echo "Linux artifact smoke failed: could not read deb package name" >&2
  exit 1
fi

sudo apt-get install -y "$DEB"
INSTALLED_BIN="$(dpkg -L "$DEB_PACKAGE" | while IFS= read -r candidate; do
  if [ -f "$candidate" ] && [ -x "$candidate" ] && file "$candidate" | grep -q 'ELF'; then
    printf '%s\n' "$candidate"
    break
  fi
done)"

if [ -z "$INSTALLED_BIN" ]; then
  echo "Linux artifact smoke failed: could not find installed executable for $DEB_PACKAGE" >&2
  dpkg -L "$DEB_PACKAGE" >&2 || true
  exit 1
fi

run_desktop_for_migration "$INSTALLED_BIN"
sudo apt-get remove -y "$DEB_PACKAGE"

rpm -qip "$RPM" >/dev/null
rpm -qlp "$RPM" >/dev/null

echo "Linux release artifact smoke passed."
