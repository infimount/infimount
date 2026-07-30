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

[ -s "$ASSET_DIR/SBOM.spdx.json" ] || fail "SBOM.spdx.json is missing or empty"
jq -e '
  (.packages[] | select(.name == "infimount_mcp")) as $sidecar |
  ($sidecar.versionInfo | type == "string" and length > 0) and
  ($sidecar.hasFiles | type == "array" and length >= 3) and
  ([
    $sidecar.hasFiles[] as $id |
    .files[] |
    select(.SPDXID == $id and any(.checksums[]?; .algorithm == "SHA256" and (.checksumValue | length == 64)))
  ] | length >= 3)
' "$ASSET_DIR/SBOM.spdx.json" >/dev/null \
  || fail "SBOM.spdx.json does not cover three checksummed infimount_mcp platform sidecars"

mapfile -t checksum_assets < <(
  find "$ASSET_DIR" -maxdepth 1 -type f -printf '%f\n' \
    | grep -Ev '^(SHA256SUMS\.txt|.*\.sha256)$' \
    | sort
)
[ "${#checksum_assets[@]}" -gt 0 ] || fail "no release payloads found"

for asset in "${checksum_assets[@]}"; do
  awk -v asset="$asset" '$2 == asset { found = 1 } END { exit !found }' "$ASSET_DIR/SHA256SUMS.txt" \
    || fail "SHA256SUMS.txt is missing entry for $asset"
  [ -s "$ASSET_DIR/$asset.sha256" ] || fail "$asset.sha256 is missing or empty"
  awk -v asset="$asset" '$2 == asset { found = 1 } END { exit !found }' "$ASSET_DIR/$asset.sha256" \
    || fail "$asset.sha256 does not reference $asset"
done

(
  cd "$ASSET_DIR"
  sha256sum -c SHA256SUMS.txt >/dev/null
  for asset in "${checksum_assets[@]}"; do
    sha256sum -c "$asset.sha256" >/dev/null
  done
)

jq -e '.version and (.platforms | type == "object" and length > 0)' "$ASSET_DIR/latest.json" >/dev/null \
  || fail "latest.json must include version and at least one platform"

printf 'Release asset check passed for %s.\n' "$ASSET_DIR"
