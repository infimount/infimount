#!/usr/bin/env node
/**
 * RUSTSEC-2026-0462 is limited to rmcp's *client* Streamable HTTP transports.
 * Infimount's current rmcp 1.x usage is stdio and loopback HTTP *server* only.
 *
 * This assertion gates the precise advisory exception in deps-audit.yml.
 * Never use it to waive unrelated vulnerabilities. See issue #231.
 */
import { execFileSync } from "node:child_process";
import { strict as assert } from "node:assert";
import { readFileSync } from "node:fs";

const VULNERABLE_CLIENT_FEATURES = [
  "client",
  "transport-streamable-http-client",
  "transport-streamable-http-client-reqwest",
  "transport-streamable-http-client-unix-socket",
];

function assertServerOnlyRmcp(metadata) {
  const crates = metadata.packages.filter((pkg) => pkg.name === "rmcp");
  assert.equal(crates.length, 1, "Exactly one resolved rmcp crate is required");
  const pkg = crates[0];
  // When the SDK moves to 3.9.0+, remove this exception and this guard,
  // rather than carrying a stale waiver into an unrelated major version.
  assert.match(pkg.version, /^1\.[0-9]+\.[0-9]+$/, "rmcp migration requires removing the RustSec exception");
  const resolved = metadata.resolve?.nodes.find((node) => node.id === pkg.id);
  assert.ok(resolved, "Resolved rmcp features must be available");
  assert.ok(resolved.features.includes("server"), "rmcp server capability must be explicitly resolved");
  assert.ok(
    resolved.features.includes("transport-streamable-http-server"),
    "Infimount's loopback HTTP MCP server transport must remain resolved",
  );
  for (const feature of VULNERABLE_CLIENT_FEATURES) {
    assert.ok(
      !resolved.features.includes(feature),
      `RUSTSEC-2026-0462 exception is unsafe: rmcp feature "${feature}" is enabled`,
    );
  }
  return pkg.version;
}

if (process.argv.includes("--self-test")) {
  const sample = (features, version = "1.8.0") => ({
    packages: [{ id: "rmcp-test-id", name: "rmcp", version }],
    resolve: { nodes: [{ id: "rmcp-test-id", features }] },
  });
  const safe = ["default", "server", "transport-io", "transport-streamable-http-server"];
  assert.equal(assertServerOnlyRmcp(sample(safe)), "1.8.0");
  for (const feature of VULNERABLE_CLIENT_FEATURES) {
    assert.throws(() => assertServerOnlyRmcp(sample([...safe, feature])));
  }
  assert.throws(() => assertServerOnlyRmcp(sample(safe, "3.9.0")));
  assert.throws(() => assertServerOnlyRmcp(sample(["default", "server"])));
  console.log("rmcp scoped-audit guard self-test passed");
} else {
  const cargo = readFileSync("crates/mcp/Cargo.toml", "utf8");
  assert.match(cargo, /rmcp\s*=\s*\{\s*version\s*=\s*"1\.4\.0"/);
  const raw = execFileSync("cargo", ["metadata", "--locked", "--format-version", "1"], {
    encoding: "utf8",
    maxBuffer: 64 * 1024 * 1024,
    stdio: ["ignore", "pipe", "inherit"],
  });
  const version = assertServerOnlyRmcp(JSON.parse(raw));
  console.log(`rmcp ${version}: no Streamable HTTP client features resolved (RUSTSEC-2026-0462 exception scope verified)`);
}
