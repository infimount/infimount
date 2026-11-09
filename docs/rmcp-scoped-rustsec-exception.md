# Scoped exception for RUSTSEC-2026-0462 (rmcp)

Issued 2026-11-09. The rmcp 1.8.0 built-in Streamable HTTP *clients*
buffer HTTP JSON/error responses without a byte cap. Patched in rmcp 3.9.0.

Infimount currently uses only rmcp's server and stdio transports. The
`crates/mcp/Cargo.toml` dependency enables `transport-io` and
`transport-streamable-http-server`; source uses `StreamableHttpService`,
`LocalSessionManager` and `rmcp::transport::stdio`. No Streamable HTTP
client is compiled into Infimount under the resolved Cargo feature graph.

The dependency audit remains blocking for all advisories **except**
RUSTSEC-2023-0071 (preexisting RSA exception) and RUSTSEC-2026-0462.
The latter is allowed only after
`scripts/check-rmcp-client-transport-disabled.mjs` verifies the
**resolved** rmcp 1.x feature set from `cargo metadata --locked`.
It fails if any client feature, including generic Streamable HTTP,
reqwest or Unix socket clients, becomes active, or if rmcp upgrades
major version without removing the exception. The guard has independent
positive and negative fixtures in `--self-test`.

This is a **conditional risk exception**, not a claim the affected crate
has been patched. Issue #231 tracks migrating Infimount's MCP server
and stdio integrations to rmcp >=3.9.0, then removing the exception.
The existing loopback-only bearer-authenticated server constraints remain.

Primary advisory: https://rustsec.org/advisories/RUSTSEC-2026-0462.html
Tracking: https://github.com/infimount/infimount/issues/231
