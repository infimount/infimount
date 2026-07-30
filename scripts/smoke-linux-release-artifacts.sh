#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
ARTIFACT_DIR="${1:-$ROOT_DIR/out/linux}"
ARTIFACT_DIR="$(cd "$ARTIFACT_DIR" && pwd)"
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

  setsid xvfb-run -a env \
    HOME="$tmp_home" \
    XDG_CONFIG_HOME="$tmp_home/.config" \
    APPIMAGE_EXTRACT_AND_RUN=1 \
    "$command_path" >"$log_file" 2>&1 &
  local desktop_pid=$!
  local migrated=false

  for _ in $(seq 1 120); do
    if ! kill -0 "$desktop_pid" 2>/dev/null; then
      set +e
      wait "$desktop_pid"
      local status=$?
      set -e
      echo "Linux artifact smoke failed: desktop exited before migration (status $status): $command_path" >&2
      tail -n 200 "$log_file" >&2 || true
      rm -rf "$tmp_home" "$log_file"
      exit 1
    fi
    if [ -s "$tmp_home/.infimount/storages.json" ] \
      && grep -Eq '"name"[[:space:]]*:[[:space:]]*"Linux Artifact Smoke Home"' "$tmp_home/.infimount/storages.json"; then
      migrated=true
      break
    fi
    sleep 0.5
  done

  if [ "$migrated" != true ]; then
    echo "Linux artifact smoke failed: desktop stayed running but migration did not complete: $command_path" >&2
    tail -n 200 "$log_file" >&2 || true
    kill -- -"$desktop_pid" 2>/dev/null || true
    wait "$desktop_pid" 2>/dev/null || true
    rm -rf "$tmp_home" "$log_file"
    exit 1
  fi

  kill -- -"$desktop_pid" 2>/dev/null || true
  wait "$desktop_pid" 2>/dev/null || true
  assert_migrated_storage "$tmp_home" "$log_file"
  rm -rf "$tmp_home" "$log_file"
}

require_file "$APPIMAGE"
require_file "$DEB"
require_file "$RPM"

RELEASE_TAG="${GITHUB_REF_NAME:-}"
EXPECTED_VERSION="${RELEASE_TAG#v}"
if [[ -z "$EXPECTED_VERSION" ]]; then
  EXPECTED_VERSION="$(node -p "require('$ROOT_DIR/apps/desktop/package.json').version")"
fi
assert_bundled_sidecar() {
  local tree=$1
  local sidecar
  sidecar="$(find "$tree" -type f -name 'mcp*' -perm -111 | sort | head -n 1)"
  if [ -z "$sidecar" ]; then
    echo "Linux artifact smoke failed: bundled MCP sidecar not found in $tree" >&2
    exit 1
  fi
  if [ "$("$sidecar" --version)" != "infimount_mcp $EXPECTED_VERSION" ]; then
    echo "Linux artifact smoke failed: bundled MCP sidecar version mismatch" >&2
    exit 1
  fi
}

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
APPIMAGE_EXTRACT_DIR="$(mktemp -d)"
(
  cd "$APPIMAGE_EXTRACT_DIR"
  "$APPIMAGE" --appimage-extract >/dev/null
)
assert_bundled_sidecar "$APPIMAGE_EXTRACT_DIR/squashfs-root"
rm -rf "$APPIMAGE_EXTRACT_DIR"
run_desktop_for_migration "$APPIMAGE"

dpkg-deb --info "$DEB" >/dev/null
DEB_EXTRACT_DIR="$(mktemp -d)"
dpkg-deb -x "$DEB" "$DEB_EXTRACT_DIR"
assert_bundled_sidecar "$DEB_EXTRACT_DIR"
rm -rf "$DEB_EXTRACT_DIR"
DEB_PACKAGE="$(dpkg-deb --field "$DEB" Package)"
if [ -z "$DEB_PACKAGE" ]; then
  echo "Linux artifact smoke failed: could not read deb package name" >&2
  exit 1
fi

sudo apt-get install -y "$DEB"
INSTALLED_BIN="$(dpkg -L "$DEB_PACKAGE" | while IFS= read -r candidate; do
  basename="$(basename "$candidate")"
  if [ -f "$candidate" ] \
    && [ -x "$candidate" ] \
    && [[ "$basename" =~ ^[Ii]nfimount$ ]] \
    && [[ ! "$basename" =~ [Mm][Cc][Pp] ]] \
    && file "$candidate" | grep -q 'ELF'; then
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
RPM_EXTRACT_DIR="$(mktemp -d)"
(
  cd "$RPM_EXTRACT_DIR"
  rpm2cpio "$RPM" | cpio -idm --quiet
)
assert_bundled_sidecar "$RPM_EXTRACT_DIR"
rm -rf "$RPM_EXTRACT_DIR"

for updater in "$ARTIFACT_DIR"/updater/*.tar.gz "$ARTIFACT_DIR"/updater/*.zip "$ARTIFACT_DIR"/updater/*.AppImage; do
  [ -f "$updater" ] || continue
  UPDATER_EXTRACT_DIR="$(mktemp -d)"
  case "$updater" in
    *.tar.gz) tar -xzf "$updater" -C "$UPDATER_EXTRACT_DIR" ;;
    *.zip) unzip -q "$updater" -d "$UPDATER_EXTRACT_DIR" ;;
    *.AppImage) cp "$updater" "$UPDATER_EXTRACT_DIR/updater.AppImage" ;;
  esac
  NESTED_APPIMAGE="$(find "$UPDATER_EXTRACT_DIR" -type f -name '*.AppImage' | head -n 1)"
  if [ -n "$NESTED_APPIMAGE" ]; then
    chmod +x "$NESTED_APPIMAGE"
    (
      cd "$UPDATER_EXTRACT_DIR"
      "$NESTED_APPIMAGE" --appimage-extract >/dev/null
    )
    assert_bundled_sidecar "$UPDATER_EXTRACT_DIR/squashfs-root"
  else
    assert_bundled_sidecar "$UPDATER_EXTRACT_DIR"
  fi
  rm -rf "$UPDATER_EXTRACT_DIR"
done

echo "Linux release artifact smoke passed."
