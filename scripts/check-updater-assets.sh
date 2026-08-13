#!/usr/bin/env bash
set -euo pipefail

ASSET_DIR="${1:-release-assets}"
EXPECTED_VERSION="${2:-${GITHUB_REF_NAME#v}}"

fail() {
  echo "updater asset check failed: $*" >&2
  exit 1
}

[ -d "$ASSET_DIR" ] || fail "asset directory not found: $ASSET_DIR"
[ -s "$ASSET_DIR/latest.json" ] || fail "latest.json is missing or empty"
[ -n "$EXPECTED_VERSION" ] || fail "expected version is empty"

jq -e --arg version "$EXPECTED_VERSION" --arg tag "v$EXPECTED_VERSION" '
  .version == $version and
  (.platforms | type == "object" and length >= 3) and
  ([.platforms | keys[] | startswith("linux-")] | any) and
  ([.platforms | keys[] | startswith("darwin-")] | any) and
  ([.platforms | keys[] | startswith("windows-")] | any) and
  ([.platforms[] |
    (.signature | type == "string" and length > 0) and
    (.url | type == "string" and startswith("https://github.com/infimount/infimount/releases/download/" + $tag + "/")) and
    (.url | test("/[^/?#]+$"))
  ] | all)
' "$ASSET_DIR/latest.json" >/dev/null || fail "latest.json schema, version, platform coverage, signature, or URL is invalid"

while IFS=$'\t' read -r platform url embedded_signature; do
  asset="${url##*/}"
  [ -s "$ASSET_DIR/$asset" ] || fail "$platform references missing updater payload: $asset"
  [ -s "$ASSET_DIR/$asset.sig" ] || fail "$platform updater signature file is missing: $asset.sig"
  file_signature="$(tr -d '\r\n' < "$ASSET_DIR/$asset.sig")"
  [ "$embedded_signature" = "$file_signature" ] \
    || fail "$platform embedded signature does not match $asset.sig"
done < <(jq -r '.platforms | to_entries[] | [.key, .value.url, .value.signature] | @tsv' "$ASSET_DIR/latest.json")

printf 'Updater asset check passed for %s (%s).\n' "$ASSET_DIR" "$EXPECTED_VERSION"
