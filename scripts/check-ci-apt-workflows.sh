#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

# All Ubuntu CI workflows must use the common mirror-aware, time-bounded APT
# installer. Raw apt-get can hang for the life of a runner on slow mirrors.
if grep -nE '(^|[[:space:]])(sudo[[:space:]]+)?apt-get[[:space:]]+(update|install)([[:space:]]|$)' .github/workflows/*.yml; then
  echo "Unbounded apt-get operation found in a GitHub Actions workflow." >&2
  echo "Use bash scripts/ci-apt-install.sh instead." >&2
  exit 1
fi

if ! grep -Fq 'run: bash scripts/ci-apt-install.sh ' .github/workflows/coverage.yml; then
  echo "Rust coverage must use the shared bounded APT installer." >&2
  exit 1
fi
