#!/usr/bin/env bash
set -euo pipefail

ASSET_DIR="${1:-release-assets}"

fail() {
  echo "release asset check failed: $*" >&2
  exit 1
}

[ -d "$ASSET_DIR" ] || fail "asset directory not found: $ASSET_DIR"

required_assets=(
  "Infimount-amd64.deb"
  "Infimount-x86_64.rpm"
  "Infimount-x86_64.AppImage"
  "Infimount.dmg"
  "Infimount.msi"
  "Infimount-setup.exe"
  "install.sh"
  "install.ps1"
  "latest.json"
  "SHA256SUMS.txt"
)

for asset in "${required_assets[@]}"; do
  [ -s "$ASSET_DIR/$asset" ] || fail "$asset is missing or empty"
done

checksum_assets=(
  "Infimount-amd64.deb"
  "Infimount-x86_64.rpm"
  "Infimount-x86_64.AppImage"
  "Infimount.dmg"
  "Infimount.msi"
  "Infimount-setup.exe"
  "install.sh"
  "install.ps1"
)

for asset in "${checksum_assets[@]}"; do
  grep -Eq "[[:space:]]${asset}$" "$ASSET_DIR/SHA256SUMS.txt" \
    || fail "SHA256SUMS.txt is missing entry for $asset"
  [ -s "$ASSET_DIR/$asset.sha256" ] || fail "$asset.sha256 is missing or empty"
  grep -Eq "[[:space:]]${asset}$" "$ASSET_DIR/$asset.sha256" \
    || fail "$asset.sha256 does not reference $asset"
done

(
  cd "$ASSET_DIR"
  sha256sum -c SHA256SUMS.txt >/dev/null
  for asset in "${checksum_assets[@]}"; do
    sha256sum -c "$asset.sha256" >/dev/null
  done
)

jq -e '.version and (.platforms | length > 0)' "$ASSET_DIR/latest.json" >/dev/null \
  || fail "latest.json must include version and at least one platform"

[ -s "$ASSET_DIR/SBOM.spdx.json" ] || echo "warning: SBOM.spdx.json not present yet; run this check after SBOM generation for final validation" >&2

printf 'Release asset check passed for %s.\n' "$ASSET_DIR"
