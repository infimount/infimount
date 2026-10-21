# Claude Code Agent Task adapter: A4 security contract

Status: **experimental, not enabled in the desktop**, pending real packaged qualification. `claude_handoff.rs` builds the deny-first MCP-only launch vector. The private `claude_runner.rs` and `claude_execution.rs` now contain a supervised invocation and A2 evidence path, but these are not registered as Tauri commands or exposed to users, and no real authenticated Claude execution has been validated yet.

## Supported CLI envelope

The contract currently requires Claude Code **v2.1.259 or newer**, because `--restricted` and `--permission-prompts none` must both be available. Versions are a necessary gate, not sufficient proof.

The plan uses these explicit controls:

| Control | Purpose |
| --- | --- |
| `--bare` | Suppress automatic project/user customizations, plugins, hooks, skills and MCP discovery |
| `--restricted` | Suppress host command execution and load only managed/explicit settings |
| `--tools ""` | Disable the built-in Bash, Read, Edit and other file/host tools |
| `--mcp-config <inline JSON>` + `--strict-mcp-config` | Load exactly the one task-specific Infimount MCP server |
| `--allowedTools` | Allow exactly six task-scoped MCP tool names |
| `--permission-mode dontAsk` + `--permission-prompts none` | Deny unattended permission requests |
| `--no-session-persistence` | Avoid saving a resumable session by default |
| `--disable-slash-commands` + `--no-chrome` | Suppress additional commands/browser integration |
| `--max-turns 24` | Bound agent iteration independently of the future process timeout |
| `--output-format json` | Enable a later supervised caller to parse a structured, bounded outcome |

Allowed MCP tools: `list_dir`, `stat_path`, `read_file`, `search_paths`, `write_file`, `mkdir`. The existing task MCP server still enforces reads inside the task and writes only inside `outputs/`, even if the model requests something else. **The model has no publication capability.**

The test contract rejects unrecognized/version-incompatible clients, relative sidecar paths, unbounded task prompts, additional MCP servers or tools, and missing/altered confinement flags.

## Supervised executor implementation (not user-enabled)

The internal A4 runner is now wired to the accepted launch contract:

- Canonicalize, SHA-256 hash, and version-probe the configured Claude executable; revalidate the exact bytes immediately before launching.
- Use a private neutral current directory and start the CLI directly, not through an arbitrary shell.
- Remove inherited environment variables, retaining only the small login/runtime set for an existing local Claude authentication. Raw `ANTHROPIC_API_KEY` and other cloud tokens are **not** forwarded by this implementation.
- Constrain execution to at most 12 minutes, terminate the dedicated Unix process group on cancellation, timeout, or exit, and cap captured stdout/stderr. Raw model responses and raw errors are never put into execution evidence.
- Require a zero process exit code **and** a valid bounded Claude JSON result with `is_error=false`. An exit code alone is not evidence of successful agent work.
- Verify task output paths, bytes and hashes through OpenDAL. Reuse the A2 private store with distinct prepared/running/terminal records and safe error codes. A completed CLI that produced no verified outputs is recorded as failed, not succeeded.
- Refuse rerunning into a task that already has output files. Output review and create-only publication remain separate human controls.
- Windows execution is **not supported in this runner** yet; it fails closed until a job-object containment implementation is qualified. Linux/macOS still need a packaged real-client pilot before a user-facing control is added.

The runner functions are compiled and unit-tested but are currently unreachable from an application command. They are a preparatory integration, not a declaration of supported Claude Code execution.
## Qualification required before enabling

1. Build from a verified and canonicalized Claude Code executable and revalidate its digest before launch, as with the A3 Codex adapter.
2. Use a neutral private working directory, with controlled environment and no original source mount.
3. Verify the actual installed CLI accepts the entire flag combination, including explicit `--mcp-config` under `--bare` and `--restricted` and the exact `--allowedTools` argument encoding.
4. Verify that a hostile user's managed settings, plugins, MCP servers, hooks or tools cannot override the denial boundary. If managed settings can bypass it, do not enable the adapter.
5. Demonstrate a complete coding and document task through the six MCP tools, and prove shell, host Read/Edit/Write, browser, and unrelated MCP tools are absent.
6. Supervise the owned process tree, implement bounded timeout and cancellation, and create truthful A2 execution records. A detached launcher must not claim an exit status or output digest.
7. Rehash inputs and outputs, preserve source storage digest/MCP exposure, and use the existing independent output review and create-only publication path.
8. Run real packaged regressions on Linux, macOS and Windows before advertising platform support. A unit test or static CLI flag check is not enough.

**No client adapter becomes enabled solely from its version string.** Until those requirements are proven, Codex remains the sole enabled first-class Agent Task target.

## References

- [Claude Code CLI flags](https://code.claude.com/docs/en/cli-reference)
- [Claude Code settings precedence](https://code.claude.com/docs/en/settings)
- [Claude Code permissions](https://code.claude.com/docs/en/permissions)
- [Infimount ADR 0007](adr/0007-fail-closed-local-agent-adapters.md)
- [v0.8.2 implementation plan](roadmaps/v0.8.2-governed-agent-data-plane.md)
