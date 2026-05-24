#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
COMPOSE_FILE="$ROOT_DIR/storage-simulator/docker-compose.yml"

require_command() {
  if ! command -v "$1" >/dev/null 2>&1; then
    echo "Missing required command: $1" >&2
    echo "Install simulator dependencies with scripts/install-storage-simulator-clis-linux.sh on Linux." >&2
    exit 1
  fi
}

require_command docker
require_command curl
require_command aws
require_command az

cleanup() {
  docker compose -f "$COMPOSE_FILE" down -v
}
trap cleanup EXIT

cd "$ROOT_DIR"

echo "Starting storage simulator..."
docker compose -f "$COMPOSE_FILE" up -d

echo "Bootstrapping storage simulator..."
(
  cd "$ROOT_DIR/storage-simulator"
  ./bootstrap.sh
)

echo "Verifying OpenDAL storage simulator..."
cargo run -p infimount_core --bin verify_storage

echo "Storage simulator gate passed."
