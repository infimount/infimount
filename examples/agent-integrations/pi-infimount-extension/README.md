# Pi Infimount Extension Starter

This Pi extension exposes read-first Infimount MCP tools inside Pi while keeping Infimount as the storage policy boundary. It is structured as a Pi package and can be loaded directly, installed from a local path, or packed for npm publishing.

## Tools

- `infimount_list_storages`
- `infimount_list_dir`
- `infimount_read_file`
- `infimount_search_paths`
- `infimount_generate_download_link`

The extension does not read cloud credentials directly and does not bypass Infimount MCP exposure, path policy, confirmations, sessions, or audit logging.

## Install for local testing

```bash
npm install
pi -e ./index.ts
```

Or install the package into Pi from this directory:

```bash
pi install .
```

By default the extension starts:

```bash
infimount_mcp --transport stdio
```

Override the binary path when needed:

```bash
INFIMOUNT_MCP_COMMAND=/absolute/path/to/infimount_mcp pi -e ./index.ts
```

Override args with a JSON array or simple whitespace-separated string:

```bash
INFIMOUNT_MCP_ARGS='["--transport","stdio"]' pi -e ./index.ts
```

## Smoke test

From this directory:

```bash
npm run smoke
```

The smoke test creates a temporary local Infimount storage under a temporary `HOME`, starts `infimount_mcp` over stdio, and verifies list/read/search tool calls through MCP.

## Package dry run

```bash
npm pack --dry-run
```

The package manifest exposes `index.ts` through the `pi.extensions` field and includes only runtime package files in the tarball.
