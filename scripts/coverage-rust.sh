#!/usr/bin/env bash
set -euo pipefail

if ! command -v cargo-llvm-cov >/dev/null 2>&1; then
  echo "cargo-llvm-cov is not installed." >&2
  echo "Install it with: cargo install cargo-llvm-cov" >&2
  exit 1
fi

mkdir -p coverage/rust
cargo llvm-cov --workspace --lcov --output-path coverage/rust/lcov.info
