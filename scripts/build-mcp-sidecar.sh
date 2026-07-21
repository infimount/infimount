#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$SCRIPT_DIR/.."
BINARIES_DIR="$PROJECT_ROOT/apps/desktop/src-tauri/binaries"

echo "Building infimount_mcp sidecar..."
cargo build -p infimount_mcp --release "$@"

TARGET="${TARGET:-$(rustc -vV | grep host | awk '{print $2}')}"
EXT=""
case "$TARGET" in
  *windows*) EXT=".exe" ;;
esac

mkdir -p "$BINARIES_DIR"
cp "$PROJECT_ROOT/target/release/infimount_mcp${EXT}" "$BINARIES_DIR/mcp-${TARGET}${EXT}"
echo "MCP sidecar copied to binaries/mcp-${TARGET}${EXT}"
