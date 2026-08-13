#!/usr/bin/env bash
set -euo pipefail

TREE=${1:?usage: verify-packaged-sidecar.sh EXTRACTED_TREE EXPECTED_VERSION [macos-signed]}
EXPECTED_VERSION=${2:?usage: verify-packaged-sidecar.sh EXTRACTED_TREE EXPECTED_VERSION [macos-signed]}
TRUST_MODE=${3:-digest}

SIDECARS=()
while IFS= read -r sidecar; do SIDECARS+=("$sidecar"); done < <(find "$TREE" -type f \( -name 'mcp' -o -name 'mcp-*' \) -perm -111 | sort)
if [ "${#SIDECARS[@]}" -ne 1 ]; then
  echo "Packaged sidecar verification failed: expected exactly one executable MCP sidecar in $TREE, found ${#SIDECARS[@]}" >&2
  exit 1
fi
SIDECAR=${SIDECARS[0]}

CHECKSUMS=()
while IFS= read -r checksum; do CHECKSUMS+=("$checksum"); done < <(find "$TREE" -type f -name 'mcp.sha256' | sort)
if [ "${#CHECKSUMS[@]}" -ne 1 ]; then
  echo "Packaged sidecar verification failed: expected exactly one mcp.sha256 resource in $TREE, found ${#CHECKSUMS[@]}" >&2
  exit 1
fi
CHECKSUM_FILE=${CHECKSUMS[0]}
if [ "$TRUST_MODE" = "macos-signed" ]; then
  codesign --verify --strict "$SIDECAR"
else
  EXPECTED_SHA=$(awk 'NR == 1 { print $1 }' "$CHECKSUM_FILE")
  if [[ ! "$EXPECTED_SHA" =~ ^[0-9a-fA-F]{64}$ ]]; then
    echo "Packaged sidecar verification failed: malformed digest in $CHECKSUM_FILE" >&2
    exit 1
  fi
  if command -v sha256sum >/dev/null 2>&1; then
    ACTUAL_SHA=$(sha256sum "$SIDECAR" | awk '{print $1}')
  else
    ACTUAL_SHA=$(shasum -a 256 "$SIDECAR" | awk '{print $1}')
  fi
  ACTUAL_SHA_LOWER=$(printf '%s' "$ACTUAL_SHA" | tr '[:upper:]' '[:lower:]')
  EXPECTED_SHA_LOWER=$(printf '%s' "$EXPECTED_SHA" | tr '[:upper:]' '[:lower:]')
  if [ "$ACTUAL_SHA_LOWER" != "$EXPECTED_SHA_LOWER" ]; then
    echo "Packaged sidecar verification failed: digest mismatch for $SIDECAR" >&2
    exit 1
  fi
fi

ACTUAL_VERSION=$("$SIDECAR" --version)
if [ "$ACTUAL_VERSION" != "infimount_mcp $EXPECTED_VERSION" ]; then
  echo "Packaged sidecar verification failed: version mismatch ($ACTUAL_VERSION != infimount_mcp $EXPECTED_VERSION)" >&2
  exit 1
fi

printf 'Verified packaged sidecar %s with checksum resource %s\n' "$SIDECAR" "$CHECKSUM_FILE"
