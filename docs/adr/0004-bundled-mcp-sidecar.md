# ADR 0004: Bundled MCP Sidecar

**Status:** Accepted  
**Date:** 2026-07-18  
**Driver:** v0.8 Trust & Activation  

## Context

Users currently need a Rust toolchain to build `infimount_mcp` separately
before they can use MCP. The desktop installer ships only the Tauri
application, so the sidecar version can drift from the desktop version.
There is no mechanism to verify that the bundled binary is present,
executable, and compatible.

## Decision

Bundle a same-version `infimount_mcp` executable in all desktop installers.
Add a `SidecarLocator` to discover and verify the sidecar at runtime.
Add a build script to prepare the sidecar before packaging.

## Affected files

- `Cargo.toml` (workspace package version)
- `crates/core/Cargo.toml`
- `crates/mcp/Cargo.toml`
- `crates/mcp/src/main.rs`
- `apps/desktop/src-tauri/Cargo.toml`
- `apps/desktop/src-tauri/tauri.conf.json`
- `apps/desktop/src-tauri/src/sidecar.rs` (new)
- `apps/desktop/src-tauri/src/state.rs`
- `apps/desktop/src-tauri/src/commands/mcp.rs`
- `apps/desktop/src-tauri/binaries/.gitkeep` (new)
- `apps/desktop/package.json`
- `package.json`
- `scripts/prepare-mcp-sidecar.mjs` (new)
- `scripts/sync-release-version.mjs`
- `scripts/check-release-consistency.mjs`
- `scripts/smoke-mcp-sidecar.sh` (new)
- `.github/workflows/release.yml`
- `docs/mcp-client-setup.md`

## Contract

### Workspace versioning

Root `Cargo.toml`:

```toml
[workspace.package]
version = "0.8.0"
edition = "2021"
rust-version = "1.85"
```

All Rust packages use `version.workspace = true`, `edition.workspace = true`,
`rust-version.workspace = true`.

### Tauri bundle config

```json
{
  "bundle": {
    "externalBin": ["binaries/infimount_mcp"]
  }
}
```

### prepare-mcp-sidecar.mjs

1. Accept optional `--target <triple>`.
2. Default target: `rustc --print host-tuple`.
3. Build: `cargo build --release -p infimount_mcp --bin infimount_mcp
   --target <triple>`.
4. Copy to: `apps/desktop/src-tauri/binaries/infimount_mcp-<triple>[.exe]`.
5. Set executable permission on Unix.
6. Run `--version`.
7. Fail if reported version differs from desktop package version.
8. Print SHA-256.
9. Never commit generated binaries.

Package scripts:
```json
{
  "scripts": {
    "build:mcp-sidecar": "node scripts/prepare-mcp-sidecar.mjs",
    "test:mcp-sidecar": "bash scripts/smoke-mcp-sidecar.sh"
  }
}
```

### Sidecar CLI

```
infimount_mcp --version
infimount_mcp serve --transport stdio
infimount_mcp serve --transport http --bind 127.0.0.1 --port 7331
infimount_mcp doctor --json
infimount_mcp print-config-dir
```

Preserve compatibility with current `--transport stdio|http` syntax.

### SidecarLocator

```rust
pub struct McpSidecarInfo {
    pub path: PathBuf,
    pub exists: bool,
    pub executable: bool,
    pub version: Option<String>,
    pub compatible: bool,
}
```

Discovery order:
1. Resolve the Tauri resource directory.
2. Check known external-binary locations for the platform.
3. Check the directory containing the desktop executable.
4. Validate each candidate by running `--version` with a 3-second timeout.
5. Select only a same-version candidate.
6. Never fall back to bare `infimount_mcp` on `PATH` for generated snippets.

### Client snippets

Generated stdio JSON uses the verified absolute path:

```json
{
  "mcpServers": {
    "infimount": {
      "command": "/absolute/path/to/infimount_mcp",
      "args": ["serve", "--transport", "stdio"]
    }
  }
}
```

- Properly JSON-escape Windows backslashes and paths with spaces.
- No credentials in snippets.
- HTTP snippet uses placeholder/environment reference.

### Release workflow

Before every `pnpm tauri build`:
1. Build target-specific sidecar.
2. Verify its version.
3. Verify it is included in the bundle.
4. Run sidecar smoke test.

Post-bundle checks verify presence and version on each platform.

## Consequences

- Fresh installer users can use stdio without a source build.
- The sidecar and desktop always report the same version.
- Release gates fail when sidecar is missing, not executable, or
  version-mismatched.
- Sidecar absence or version mismatch produces a specific, actionable
  error.
