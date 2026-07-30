#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"
# Exercises the clean legacy registry migration path and its safe MCP defaults.
cargo test -p infimount state::tests::legacy_source_migration_defaults_to_not_mcp_exposed -- --exact
