#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
VERIFY="$ROOT_DIR/scripts/verify-packaged-sidecar.sh"
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT

make_fixture() {
  local name=$1
  local sidecar_dir=$2
  local resource_dir=$3
  local tree="$TMP/$name"
  mkdir -p "$tree/$sidecar_dir" "$tree/$resource_dir"
  cat > "$tree/$sidecar_dir/mcp" <<'SCRIPT'
#!/usr/bin/env bash
if [ "${1:-}" = "--version" ]; then
  echo "infimount_mcp 0.8.0-rc.1"
else
  exit 2
fi
SCRIPT
  chmod +x "$tree/$sidecar_dir/mcp"
  sha256sum "$tree/$sidecar_dir/mcp" | awk '{print $1}' > "$tree/$resource_dir/mcp.sha256"
  "$VERIFY" "$tree" 0.8.0-rc.1 >/dev/null
}

# Representative AppImage, Debian, and RPM Tauri resource layouts.
make_fixture appimage usr/bin usr/lib/infimount/binaries
make_fixture deb usr/bin usr/lib/infimount/binaries
make_fixture rpm usr/bin usr/lib/Infimount/binaries

missing="$TMP/missing"
mkdir -p "$missing/usr/bin"
cp "$TMP/appimage/usr/bin/mcp" "$missing/usr/bin/mcp"
if "$VERIFY" "$missing" 0.8.0-rc.1 >/dev/null 2>&1; then
  echo "Packaged sidecar checksum test failed: missing checksum was accepted" >&2
  exit 1
fi

tampered="$TMP/tampered"
cp -a "$TMP/appimage" "$tampered"
echo '# tampered' >> "$tampered/usr/bin/mcp"
if "$VERIFY" "$tampered" 0.8.0-rc.1 >/dev/null 2>&1; then
  echo "Packaged sidecar checksum test failed: digest mismatch was accepted" >&2
  exit 1
fi

echo "Packaged sidecar checksum fixture tests passed."
