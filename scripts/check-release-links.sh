#!/usr/bin/env bash
set -euo pipefail

BASE_URL="${INFIMOUNT_RELEASE_BASE_URL:-https://github.com/infimount/infimount/releases/latest/download}"
ASSETS=(
  "Infimount-amd64.deb"
  "Infimount-x86_64.rpm"
  "Infimount-x86_64.AppImage"
  "Infimount.dmg"
  "Infimount.msi"
  "Infimount-setup.exe"
  "install.sh"
  "install.ps1"
  "SHA256SUMS.txt"
)

for asset in "${ASSETS[@]}"; do
  url="${BASE_URL}/${asset}"
  printf 'Checking %s\n' "$url"
  curl \
    --fail \
    --silent \
    --show-error \
    --location \
    --head \
    --connect-timeout 30 \
    --max-time 60 \
    "$url" >/dev/null
done

printf 'All release links resolved.\n'
