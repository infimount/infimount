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
  "$ROOT_DIR/scripts/verify-packaged-sidecar.sh" "$tree" "$EXPECTED_VERSION"
}

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
