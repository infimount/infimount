#!/usr/bin/env bash
set -euo pipefail

# FTP remains a legacy enum value so existing configurations can fail safely,
# but v0.8 must not ship the vulnerable OpenDAL FTP adapter.
if cargo tree --workspace --all-features | grep -Eq '(^|[[:space:]])(suppaftp|opendal-service-ftp) v'; then
  echo 'FTP dependency must not be present in the v0.8 dependency graph' >&2
  exit 1
fi
if grep -q '"id": "ftp"' crates/core/storage_schemas.json; then
  echo 'FTP must not be user-selectable in storage schemas' >&2
  exit 1
fi
if grep -qE '"ftp"|"ftp":' apps/desktop/src/types/storage.ts apps/desktop/src/types/source.ts; then
  echo 'FTP must not be configurable through desktop TypeScript schemas' >&2
  exit 1
fi

echo 'FTP security disablement checks passed.'
