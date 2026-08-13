#!/usr/bin/env bash
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"; OUT="${1:?output directory}"; VERSION="${2:?version}"; tree="$OUT/package-tree"; rm -rf "$tree"; mkdir -p "$tree/usr/lib/infimount"
cat > "$tree/usr/lib/infimount/mcp" <<EOF
#!/bin/sh
echo infimount_mcp $VERSION
EOF
chmod +x "$tree/usr/lib/infimount/mcp"; sha256sum "$tree/usr/lib/infimount/mcp" | awk '{print $1 "  mcp"}' > "$tree/usr/lib/infimount/mcp.sha256"
bash "$ROOT_DIR/scripts/verify-packaged-sidecar.sh" "$tree" "$VERSION"
# The default rehearsal is toolchain-independent and does not install packages as root.
for pkg in Infimount-amd64.deb Infimount-x86_64.rpm Infimount-x86_64.AppImage; do [[ -s "$OUT/$pkg" ]] || { echo "missing fixture package $pkg" >&2; exit 1; }; done
echo 'Linux package extraction simulation passed (fixture layout; no root install).'
