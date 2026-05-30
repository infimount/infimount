#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
RELEASE_WORKFLOW="$ROOT_DIR/.github/workflows/release.yml"
POST_RELEASE_WORKFLOW="$ROOT_DIR/.github/workflows/post-release.yml"
RELEASE_GATE_SCRIPT="$ROOT_DIR/scripts/release-test-gate.sh"
RELEASING_DOC="$ROOT_DIR/docs/releasing.md"

fail() {
  echo "zero-manual release policy check failed: $*" >&2
  exit 1
}

require_file_contains() {
  local file=$1
  local needle=$2
  grep -Fq -- "$needle" "$file" || fail "$file must contain: $needle"
}

extract_job_block() {
  local job=$1
  awk -v job="  ${job}:" '
    $0 == job { in_job = 1; print; next }
    in_job && /^  [A-Za-z0-9_-]+:/ { exit }
    in_job { print }
  ' "$RELEASE_WORKFLOW"
}

require_job_needs() {
  local job=$1
  local dependency=$2
  extract_job_block "$job" | grep -Fq -- "- $dependency" \
    || fail "release job '$job' must depend on '$dependency' before building artifacts"
}

required_gates=(
  release-frontend-gate
  release-ui-gate
  release-rust-gate
  release-desktop-smoke
  release-storage-simulator
  release-consistency-gate
  release-policy-gate
)

for gate in "${required_gates[@]}"; do
  require_file_contains "$RELEASE_WORKFLOW" "  ${gate}:"
done

for build_job in build-linux build-macos build-windows; do
  for gate in "${required_gates[@]}"; do
    if [[ "$gate" == "release-policy-gate" ]]; then
      require_job_needs "$build_job" "$gate"
    elif ! extract_job_block "$build_job" | grep -Fq -- "- $gate"; then
      fail "release job '$build_job' must depend on '$gate' before building artifacts"
    fi
  done
done

required_release_commands=(
  "typecheck"
  "test:unit"
  "test:integration"
  "test:coverage:frontend"
  "test:ui"
  "check-release-consistency.mjs"
  "smoke-install-scripts.sh"
  "cargo fmt --all -- --check"
  "cargo clippy --workspace --all-targets"
  "cargo test --workspace"
  "coverage-rust.sh"
  "smoke-desktop.sh"
  "storage-simulator-gate.sh"
)

for command in "${required_release_commands[@]}"; do
  require_file_contains "$RELEASE_GATE_SCRIPT" "$command"
done

require_file_contains "$RELEASE_WORKFLOW" "smoke-linux-release-artifacts.sh"
require_file_contains "$RELEASE_WORKFLOW" "check-release-assets.sh"
require_file_contains "$RELEASE_WORKFLOW" "smoke-install-scripts.sh"
require_file_contains "$RELEASE_WORKFLOW" "check-release-consistency.mjs"

require_file_contains "$POST_RELEASE_WORKFLOW" "types: [published]"
require_file_contains "$POST_RELEASE_WORKFLOW" "check-release-links.sh"
require_file_contains "$POST_RELEASE_WORKFLOW" "check-homebrew-update.sh"
require_file_contains "$POST_RELEASE_WORKFLOW" "homebrew-infimount/dispatches"

require_file_contains "$RELEASING_DOC" "Zero manual product test execution"
require_file_contains "$RELEASING_DOC" "Manual product test execution must not be a release gate"

printf 'Zero-manual release policy check passed.\n'
