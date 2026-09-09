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
URIs: http://archive.ubuntu.com/ubuntu
Suites: jammy jammy-updates
Components: main universe
EOF

bash "$ROOT/scripts/prepare-ci-apt-sources.sh" "$TMP/sources"

test ! -e "$TMP/sources/google-chrome.list"
test -f "$TMP/sources/google-chrome.list.infimount-disabled"
grep -Fq 'dl.google.com/linux/chrome-stable/deb' \
  "$TMP/sources/google-chrome.list.infimount-disabled"

test -f "$TMP/sources/microsoft-prod.list"
test -f "$TMP/sources/ubuntu.sources"

grep -Fq 'packages.microsoft.com' "$TMP/sources/microsoft-prod.list"
grep -Fq 'archive.ubuntu.com' "$TMP/sources/ubuntu.sources"

# Idempotent on a second pass: no new source is disabled and retained sources stay intact.
bash "$ROOT/scripts/prepare-ci-apt-sources.sh" "$TMP/sources"
test -f "$TMP/sources/google-chrome.list.infimount-disabled"
test -f "$TMP/sources/microsoft-prod.list"
test -f "$TMP/sources/ubuntu.sources"

# The privileged wrapper must isolate sources first and use a bounded retry loop.
grep -Fq 'prepare-ci-apt-sources.sh' "$ROOT/scripts/ci-apt-install.sh"
grep -Fq 'for attempt in 1 2 3' "$ROOT/scripts/ci-apt-install.sh"
grep -Fq 'apt-get install -y "$@"' "$ROOT/scripts/ci-apt-install.sh"

echo "CI APT source isolation test passed."
