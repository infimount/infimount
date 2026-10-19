#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MANIFEST="$ROOT/apps/desktop/src-tauri/Cargo.toml"
LOCKFILE="$ROOT/Cargo.lock"
CONFIG="$ROOT/apps/desktop/src-tauri/tauri.conf.json"

fail() {
  echo "updater security contract failed: $*" >&2
  exit 1
}

# Tauri updater v2 accepts signed, direct MSI/NSIS EXE payloads on Windows.
# The optional 'zip' feature is only needed to support legacy v1 ZIP bundles.
# Keeping it enabled pulls in zip 4.x, which is affected by multiple RustSec
# vulnerabilities. Opt out without disabling rustls TLS or updater signing.
grep -Eq '^[[:space:]]*tauri-plugin-updater[[:space:]]*=[[:space:]]*\{[[:space:]]*version[[:space:]]*=[[:space:]]*"2\.[0-9]+\.[0-9]+"[[:space:]]*,[[:space:]]*default-features[[:space:]]*=[[:space:]]*false[[:space:]]*,[[:space:]]*features[[:space:]]*=[[:space:]]*\["rustls-tls"\][[:space:]]*\}' "$MANIFEST" \
  || fail "Tauri updater must explicitly disable default features and keep rustls-tls"

grep -Fq '"createUpdaterArtifacts": true' "$CONFIG" \
  || fail "Tauri must emit v2 updater artifacts, not legacy ZIP bundles"

if grep -Eq '^name = "zip"$' "$LOCKFILE"; then
  fail "Cargo.lock still contains a ZIP crate; updater must not resolve legacy ZIP support"
fi

if grep -Eq '^ "zip( [^"]+)?",$' "$LOCKFILE"; then
  fail "Cargo.lock still has a ZIP dependency edge"
fi

printf '%s\n' "Updater v2 security contract passed: TLS enabled, v1 ZIP disabled, signed direct artifacts retained."
