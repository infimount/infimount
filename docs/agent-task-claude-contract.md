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

The runner functions are compiled and unit-tested. An optional `claude-pilot` feature now exposes a deliberately consent-gated operator test path on Unix; ordinary builds still reject all Claude execution requests. This is not a declaration of supported Claude Code execution.
## Opt-in operator qualification build (A4 pilot)

A dedicated Cargo feature, `claude-pilot`, exposes an **experimental** prepared-task
panel only in an explicitly compiled Unix development/pilot build. It is
**disabled in normal builds** and is not part of a stable release. The standard
agent adapter list continues to describe Claude Code as not enabled.

Run only against synthetic, nonsensitive inputs, and use a separate test
account where practical.

1. Verify Claude Code is installed, authenticated, and at least version
   `2.1.259`. Find its resolved executable path (for example,
   `readlink -f "$(command -v claude)"` on Linux). Infimount hashes and
   validates the binary and its version before launching.
2. Install dependencies and prepare the bundled MCP sidecar as described in
   the developer setup. Run the desktop from the project root with
   `pnpm --dir apps/desktop tauri dev --features claude-pilot`.
   For packaged qualification use `pnpm --dir apps/desktop tauri build
   --features claude-pilot` in an isolated internal environment. **Never
   publish these opt-in pilot packages to the stable release channel.**
3. Prepare a fresh bounded Agent Task from synthetic files in the desktop UI.
   Use a Local Filesystem Agent Workspace with scoped MCP access enabled.
   Confirm the source files are not newly exposed to MCP by the operation.
4. Under the prepared task, open **Experimental Claude Code qualification**.
   Enter the absolute CLI executable path and explicitly acknowledge that
   task contents may be sent to the configured Claude inference provider.
   Without both requirements the Run action remains disabled.
5. Execute a small coding or document task; optionally test **Stop pilot**
   against a longer synthetic task. Closing the prepared-task dialog is
   intentionally blocked while its pilot is running.
6. After execution, verify its returned execution ID, terminal status,
   and file counts against the private A2 evidence at
   `<Infimount config>/agent-task-executions/<workspace-id>/<task-id>/`.
   Review outputs in the existing separate review panel; publication still
   requires its own explicit, create-only approval.
7. Perform separate adversarial runs on **fresh tasks** asking for an
   unselected source path, host Bash/Read/Edit, an unrelated MCP server,
   or write-back without review. None may gain authority. Record only
   privacy-safe pass/fail evidence, never private task bytes, bearer tokens,
   or full CLI logs.

The pilot executor inherits only a restricted set of runtime/login variables.
It does not forward arbitrary `ANTHROPIC_API_KEY`, cloud credentials, or
unrelated environment secrets. Users depending on environment-only API
authentication may need a separate, explicitly reviewed authentication design.
Do not relax environment isolation merely to make the pilot succeed.

**This opt-in feature is a test entrypoint, not qualification evidence.**
A working CLI version string, a successful synthetic task, or the compiled
pilot feature alone does not establish production safety. Before enabling
Claude Code as a default adapter, retain independent packaged Linux/macOS
qualification and implemented Windows job-object containment, verify actual
effective tool exposure, and confirm complete coding/document/data-analysis
coverage plus source immutability and independently reviewed publication.

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
