#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

mkdir -p "$TMP/sources"
cat > "$TMP/sources/google-chrome.list" <<'EOF'
deb [arch=amd64] https://dl.google.com/linux/chrome-stable/deb/ stable main
EOF
cat > "$TMP/sources/microsoft-prod.list" <<'EOF'
deb [arch=amd64] https://packages.microsoft.com/ubuntu/22.04/prod jammy main
EOF
cat > "$TMP/sources/ubuntu.sources" <<'EOF'
Types: deb
URIs: http://azure.archive.ubuntu.com/ubuntu
Suites: jammy jammy-updates
Components: main universe
EOF

first_output="$(bash "$ROOT/scripts/prepare-ci-apt-sources.sh" "$TMP/sources")"
grep -Fq 'Disabled unrelated Google Chrome APT source: google-chrome.list' <<<"$first_output"
grep -Fq 'Repointed Ubuntu Azure archive to official mirror: ubuntu.sources' <<<"$first_output"

test ! -e "$TMP/sources/google-chrome.list"
test -f "$TMP/sources/google-chrome.list.infimount-disabled"
grep -Fq 'dl.google.com/linux/chrome-stable/deb' \
  "$TMP/sources/google-chrome.list.infimount-disabled"

test -f "$TMP/sources/microsoft-prod.list"
test -f "$TMP/sources/ubuntu.sources"

grep -Fq 'packages.microsoft.com' "$TMP/sources/microsoft-prod.list"
grep -Fq 'URIs: https://archive.ubuntu.com/ubuntu' "$TMP/sources/ubuntu.sources"
! grep -Fq 'azure.archive.ubuntu.com' "$TMP/sources/ubuntu.sources"

# Idempotent on a second pass: no already-disabled source is renamed again.
second_output="$(bash "$ROOT/scripts/prepare-ci-apt-sources.sh" "$TMP/sources")"
test -z "$second_output"
test -f "$TMP/sources/google-chrome.list.infimount-disabled"
test ! -e "$TMP/sources/google-chrome.list.infimount-disabled.infimount-disabled"
test -f "$TMP/sources/microsoft-prod.list"
test -f "$TMP/sources/ubuntu.sources"

# The privileged wrapper must isolate sources first and use a bounded retry loop.
grep -Fq 'prepare-ci-apt-sources.sh' "$ROOT/scripts/ci-apt-install.sh"
grep -Fq 'for attempt in 1 2 3' "$ROOT/scripts/ci-apt-install.sh"
# APT update/install must remain bounded and installation must retry
# transient package download errors rather than hang on a slow mirror.
grep -Fq 'APT_UPDATE_TIMEOUT_SECONDS="${APT_UPDATE_TIMEOUT_SECONDS:-180}"' "$ROOT/scripts/ci-apt-install.sh"
grep -Fq 'APT_INSTALL_TIMEOUT_SECONDS="${APT_INSTALL_TIMEOUT_SECONDS:-900}"' "$ROOT/scripts/ci-apt-install.sh"
grep -Fq 'timeout --signal=TERM --kill-after=15s "${APT_INSTALL_TIMEOUT_SECONDS}s"' "$ROOT/scripts/ci-apt-install.sh"
grep -Fq 'Acquire::Retries=3' "$ROOT/scripts/ci-apt-install.sh"
grep -Fq 'Acquire::http::Timeout=45' "$ROOT/scripts/ci-apt-install.sh"
grep -Fq 'Acquire::https::Timeout=45 install -y "$@"' "$ROOT/scripts/ci-apt-install.sh"

echo "CI APT source isolation test passed."
