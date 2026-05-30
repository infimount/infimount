#!/usr/bin/env bash
set -euo pipefail

BASE_URL="${INFIMOUNT_RELEASE_BASE_URL:-https://github.com/infimount/infimount/releases/latest/download}"
ASSETS=(
  "Infimount-amd64.deb"
  "Infimount-amd64.deb.sha256"
  "Infimount-x86_64.rpm"
  "Infimount-x86_64.rpm.sha256"
  "Infimount-x86_64.AppImage"
  "Infimount-x86_64.AppImage.sha256"
  "Infimount.dmg"
  "Infimount.dmg.sha256"
  "Infimount.msi"
  "Infimount.msi.sha256"
  "Infimount-setup.exe"
  "Infimount-setup.exe.sha256"
  "install.sh"
  "install.sh.sha256"
  "install.ps1"
  "install.ps1.sha256"
  "latest.json"
  "SBOM.spdx.json"
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
    --retry 3 \
    --retry-delay 2 \
    --retry-all-errors \
    --connect-timeout 20 \
    --max-time 45 \
    "$url" >/dev/null
done

printf 'All release links resolved.\n'
