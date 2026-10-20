# Agent Task adapters (v0.8.2 A3)

**Status:** first-class Codex registry and capability detection implemented. Claude Code, OpenCode, Gemini CLI, and Pi are listed as **not implemented**; their normal MCP setup snippets do not grant them first-class Agent Task execution.

## Separation of authority

An Agent Task still belongs to Infimount, not to the selected AI client.

```text
prepared task + scoped MCP policy
           |
trusted Infimount preflight (A1)
           |
adapter capability + executable identity (A3)
           |
restricted, pinned launch plan
           |
existing Codex-only task MCP server
           |
outputs/ → independent review → explicit publication
```

The registered `codex` adapter can be marked ready only when its executable resolves to an absolute file, is executable, has a bounded SHA-256 digest, and reports recognizable version output. Client executable paths are deliberately **not returned to the UI or model**; the UI receives only a short digest prefix and sanitized version.

On Unix, discovery honors the user's login shell, matching the original Codex handoff's PATH semantics. Windows discovery uses PowerShell `Get-Command` and can probe npm `.cmd` / `.ps1` shims. All version probes are bounded and timed out, with raw command output excluded from error responses.

## Launch integrity

The existing v0.8.1 Codex `codex_arguments` builder and 104-argument golden fixture remain unchanged. The adapter constructs an explicit launch plan, re-hashes the executable immediately before spawning, and generates a temporary terminal launcher that checks the planned binary's SHA-256 again.

- Linux/macOS launchers execute the pinned binary only after SHA-256 validation.
- The Windows launcher checks the PowerShell-resolved executable path and SHA-256 before invoking Codex.
- Missing/changed executables cause a **refusal**, not a fallback to a different program.
- The neutral working directory remains `infimount-codex-<uuid>`, private on Unix, and is cleaned up by the existing launcher.
- `features.code_mode_host=true` remains the narrow MCP routing compatibility exception while code mode, shell, browser, plugins, project instructions, and other ambient tools stay disabled.
- No source storage credentials or original host source paths enter the task prompt.

Hashing a file path before opening and launching it is a best-effort local integrity check; it is **not** an OS-handle-level defense against an already-compromised local user account. The release gate still requires real packaged Codex pilot validation on supported platforms.

## UI

The prepared-task dialog queries `list_agent_task_adapters` and shows a disabled list of unsupported clients. Only verified Codex can be selected for first-class execution. A stale or missing detection never grants authority; launching triggers a **fresh capability check** and verifies the executable again.

The original `launch_agent_task_in_codex` Tauri command remains compatible.

## Limitations

This phase does not convert Codex's historical interactive terminal into a supervised agent job. The operating system terminal is launched detached, so Infimount cannot yet truthfully claim that it captured the exact completion time, exit status, process-tree cancellation or final output digest. The private A2 `ExecutionRecordStore` is ready, but **no misleading succeeded/running records are written** for the detached legacy handoff.

An explicit supervised execution lifecycle (process ownership, cancellation, record transitions) and additional fail-closed agent adapters require separate packaged compatibility work. Nothing in A3 changes review-before-publication or the create-only write-back boundary.

No new stable tag is created.
