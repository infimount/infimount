#!/usr/bin/env bash
set -euo pipefail

echo "=== v0.8 baseline: install & build ==="
pnpm install --frozen-lockfile

echo "=== v0.8 baseline: desktop lint ==="
pnpm --dir apps/desktop lint

echo "=== v0.8 baseline: desktop typecheck ==="
pnpm --dir apps/desktop typecheck

echo "=== v0.8 baseline: desktop unit tests ==="
pnpm --dir apps/desktop test:unit

echo "=== v0.8 baseline: desktop integration tests ==="
pnpm --dir apps/desktop test:integration

echo "=== v0.8 baseline: cargo fmt ==="
cargo fmt --all -- --check

echo "=== v0.8 baseline: cargo clippy ==="
cargo clippy --workspace --all-targets -- -D warnings \
  -A clippy::result_large_err \
  -A clippy::needless_borrows_for_generic_args

echo "=== v0.8 baseline: cargo test ==="
cargo test --workspace

echo "=== v0.8 baseline: all checks passed ==="
