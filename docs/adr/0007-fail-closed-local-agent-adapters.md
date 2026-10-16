# 0007 — Fail-closed local Agent Task adapters

Date: 2026-10-16

## Status

Accepted for v0.8.2 implementation.

## Context

v0.8.1 validated the first-class Codex handoff with a neutral working directory, a read-only sandbox, and a unique task-scoped Infimount MCP server. Different agent CLIs do not share the same flags, ambient configuration, built-in tools, or execution modes. A single generic command-line template risks silently granting more authority when a client updates.

## Decision

1. Keep task/workspace/source authorization in Infimount, not in any adapter.
2. Every adapter reports a versioned capability assessment before launching. Unsupported or unknown security flags cause a fail-closed result, never a permissive fallback.
3. Resolve, canonicalize, and hash the local client executable. Revalidate the executable identity immediately before launch to limit substitution between detection and execution.
4. Use an empty, secured neutral working directory. Never give the agent original source-storage or workspace host paths. Allow task data access only through its task-specific MCP server.
5. Supply an explicit exact MCP allowlist and deny all unrelated MCP servers. Disable local shell/exec/edit/web/browser/plugins/skills/project instructions and any client-specific ambient capabilities that could escape the task boundary.
6. Keep task MCP read tools confined to the prepared task and all writes confined to `outputs/`. The six allowed tools are `list_dir`, `stat_path`, `read_file`, `search_paths`, `write_file`, and `mkdir`. No adapter may expand this set.
7. The existing Codex `features.code_mode_host=true` exception is permitted only to route MCP while `features.code_mode=false` and the ambient execution surfaces remain disabled. It is not a general code-execution grant.
8. Capture status, exit code, timeout/cancellation, client version, executable hash, and bounded redacted diagnostics. Do not capture raw stdout/stderr indefinitely or treat arbitrary agent text as reliable audit evidence.
9. Kill the entire owned process tree on timeout/cancel. A launched client must not outlive its authority indefinitely.
10. Advertise a client as supported only after packaged real-client regression proves its boundary on the actual supported client version. A compatible MCP configuration snippet is not proof of full first-class task handoff safety.

## Rejected alternatives

- Assume a CLI's default sandbox is safe: defaults and user/project configs change.
- Permit fallback to broad access when restrictive launch flags fail: defeats the product's value.
- Give clients direct source-storage credentials or host mounts: bypasses Infimount.
- Let adapters implement their own publication: splits a single review/write-back contract.

## Consequences and tests

A1 establishes golden v0.8.1 Codex parity before refactoring; A4 proves the abstraction with Claude Code. Further clients remain experimental until denied ambient tools, exact MCP scope, source immutability, non-overwrite outputs, and cancellation are demonstrated. Capability failures must be visible and actionable without weakening policy.
