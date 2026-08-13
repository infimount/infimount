#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"
# Core cryptographic round trips plus command-layer restore validation.
cargo test -p infimount_core backup
cargo test -p infimount commands::backup
