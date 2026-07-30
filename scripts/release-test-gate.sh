#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DESKTOP_DIR="$ROOT_DIR/apps/desktop"

run() {
  printf '\n\033[1;36m==> %s\033[0m\n' "$*"
  "$@"
}

SKIP_UI="${INFIMOUNT_RELEASE_GATE_SKIP_UI:-0}"
SKIP_DESKTOP_SMOKE="${INFIMOUNT_RELEASE_GATE_SKIP_DESKTOP_SMOKE:-0}"
SKIP_RUST_COVERAGE="${INFIMOUNT_RELEASE_GATE_SKIP_RUST_COVERAGE:-0}"
SKIP_STORAGE_SIMULATOR="${INFIMOUNT_RELEASE_GATE_SKIP_STORAGE_SIMULATOR:-0}"

run pnpm --dir "$DESKTOP_DIR" lint
run pnpm --dir "$DESKTOP_DIR" typecheck
run pnpm --dir "$DESKTOP_DIR" test:unit
run pnpm --dir "$DESKTOP_DIR" test:integration
run pnpm --dir "$DESKTOP_DIR" test:coverage:frontend

if [ "$SKIP_UI" != "1" ]; then
  run pnpm --dir "$DESKTOP_DIR" test:ui
else
  echo "Skipping Playwright UI tests because INFIMOUNT_RELEASE_GATE_SKIP_UI=1"
fi

run pnpm --dir "$DESKTOP_DIR" build

cd "$ROOT_DIR"
run node "$ROOT_DIR/scripts/check-release-consistency.mjs" "$(node -p 'require("./apps/desktop/package.json").version')"
run node "$ROOT_DIR/scripts/check-feature-docs.mjs"
run node "$ROOT_DIR/scripts/check-upgrade-fixtures.mjs"
run "$ROOT_DIR/scripts/smoke-install-scripts.sh"
run cargo fmt --all -- --check
run cargo clippy --workspace --all-targets -- -D warnings -A clippy::result_large_err -A clippy::needless_borrows_for_generic_args
run cargo test --workspace
run "$ROOT_DIR/scripts/smoke-activation.sh"
run "$ROOT_DIR/scripts/smoke-secret-migration.sh"
run "$ROOT_DIR/scripts/smoke-backup-restore.sh"

if [ "$SKIP_RUST_COVERAGE" != "1" ]; then
  run "$ROOT_DIR/scripts/coverage-rust.sh"
else
  echo "Skipping Rust coverage because INFIMOUNT_RELEASE_GATE_SKIP_RUST_COVERAGE=1"
fi

if [ "$SKIP_DESKTOP_SMOKE" != "1" ]; then
  run "$ROOT_DIR/scripts/smoke-desktop.sh"
else
  echo "Skipping desktop smoke because INFIMOUNT_RELEASE_GATE_SKIP_DESKTOP_SMOKE=1"
fi

if [ "$SKIP_STORAGE_SIMULATOR" != "1" ]; then
  run "$ROOT_DIR/scripts/storage-simulator-gate.sh"
else
  echo "Skipping storage simulator because INFIMOUNT_RELEASE_GATE_SKIP_STORAGE_SIMULATOR=1"
fi

run "$ROOT_DIR/scripts/check-zero-manual-release-gate.sh"

printf '\n\033[1;32mRelease test gate passed.\033[0m\n'
