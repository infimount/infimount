#!/usr/bin/env node
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { spawnSync } from "node:child_process";
import { Client } from "@modelcontextprotocol/sdk/client/index.js";
import { StdioClientTransport } from "@modelcontextprotocol/sdk/client/stdio.js";

const repoRoot = path.resolve(import.meta.dirname, "../../..");
const tempRoot = fs.mkdtempSync(path.join(os.tmpdir(), "infimount-pi-ext-"));
const home = path.join(tempRoot, "home");
const storageRoot = path.join(tempRoot, "storage");
fs.mkdirSync(path.join(home, ".infimount"), { recursive: true });
fs.mkdirSync(path.join(storageRoot, "docs"), { recursive: true });
fs.writeFileSync(path.join(storageRoot, "docs", "hello.txt"), "hello from Infimount MCP\n");

const now = new Date().toISOString();
fs.writeFileSync(
  path.join(home, ".infimount", "storages.json"),
  JSON.stringify(
    [
      {
        id: "11111111-1111-4111-8111-111111111111",
        name: "SmokeLocal",
        backend: "local",
        config: { root: storageRoot },
        enabled: true,
        mcp_exposed: true,
        read_only: true,
        created_at: now,
        updated_at: now,
      },
    ],
    null,
    2,
  ),
);

let command = process.env.INFIMOUNT_MCP_COMMAND;
let args = ["--transport", "stdio"];

if (!command) {
  const build = spawnSync("cargo", ["build", "-q", "-p", "infimount_mcp"], {
    cwd: repoRoot,
    env: process.env,
    stdio: "inherit",
  });
  if (build.status !== 0) {
    throw new Error("failed to build infimount_mcp for smoke test");
  }
  command = path.join(repoRoot, "target", "debug", process.platform === "win32" ? "infimount_mcp.exe" : "infimount_mcp");
}

const transport = new StdioClientTransport({
  command,
  args,
  cwd: repoRoot,
  env: {
    ...process.env,
    HOME: home,
    RUST_LOG: "error",
  },
  stderr: "pipe",
});

transport.stderr?.on("data", (chunk) => {
  const text = chunk.toString();
  if (text.trim()) process.stderr.write(text);
});

const client = new Client({ name: "pi-infimount-extension-smoke", version: "0.1.0" });

function textContent(result) {
  return (result.content ?? [])
    .map((item) => (item.type === "text" ? item.text : JSON.stringify(item)))
    .join("\n");
}

try {
  await client.connect(transport);

  const tools = await client.listTools();
  for (const name of ["list_storages", "list_dir", "read_file", "search_paths"]) {
    if (!tools.tools.some((tool) => tool.name === name)) {
      throw new Error(`missing Infimount MCP tool: ${name}`);
    }
  }

  const root = await client.callTool({ name: "list_dir", arguments: { path: "/", limit: 20 } });
  if (!textContent(root).includes("SmokeLocal")) throw new Error("root listing did not include SmokeLocal");

  const listed = await client.callTool({ name: "list_dir", arguments: { path: "/SmokeLocal/docs", limit: 20 } });
  if (!textContent(listed).includes("hello.txt")) throw new Error("directory listing did not include hello.txt");

  const read = await client.callTool({ name: "read_file", arguments: { path: "/SmokeLocal/docs/hello.txt" } });
  if (!textContent(read).includes("hello from Infimount MCP")) throw new Error("read_file did not return fixture content");

  const search = await client.callTool({
    name: "search_paths",
    arguments: { path: "/SmokeLocal", pattern: "hello", max_results: 20 },
  });
  if (!textContent(search).includes("hello.txt")) throw new Error("search_paths did not find hello.txt");

  console.log("Pi Infimount extension MCP smoke passed.");
} finally {
  await client.close().catch(() => undefined);
  fs.rmSync(tempRoot, { recursive: true, force: true });
}
