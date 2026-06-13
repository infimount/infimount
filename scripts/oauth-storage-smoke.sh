#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

has_gdrive=false
has_onedrive=false

if [ -n "${INFIMOUNT_GDRIVE_ACCESS_TOKEN:-}" ] || [ -n "${INFIMOUNT_GDRIVE_REFRESH_TOKEN:-}" ]; then
  has_gdrive=true
fi
if [ -n "${INFIMOUNT_ONEDRIVE_ACCESS_TOKEN:-}" ] || [ -n "${INFIMOUNT_ONEDRIVE_REFRESH_TOKEN:-}" ]; then
  has_onedrive=true
fi

if [ "$has_gdrive" = false ] && [ "$has_onedrive" = false ]; then
  echo "No OAuth storage credentials configured; skipping optional OAuth storage smoke."
  echo "Set INFIMOUNT_GDRIVE_* and/or INFIMOUNT_ONEDRIVE_* variables to run live provider checks."
  exit 0
fi

echo "Running optional OAuth storage smoke. Secret values will not be printed."
cargo run -p infimount_core --bin verify_oauth_storage
