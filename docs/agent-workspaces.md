# Agent Workspaces

Agent Workspaces define a storage-scoped MCP boundary for agents and Agent Tasks. The workspace identity is the selected storage, its managed workspace folder, and the access profile. Agent-specific starter files are optional convenience content, not part of the authorization model.

## Create a workspace

1. Add a writable storage with a valid backend configuration.
2. Open **Agent Workspaces**.
3. Choose the storage and enter a workspace name.
4. Leave agent writes off for read-only access, or explicitly enable writes when the workspace must host Agent Task outputs.
5. Optionally choose starter-note files. The default is a plain workspace without agent-specific memory files.

The desktop derives the workspace folder from the name under `/agent-workspaces/<name>` and shows that storage-relative location before creation. This is a path inside the selected storage, not another host filesystem root to configure.

Normal desktop creation always creates the workspace-managed MCP path rule. The policy choice is not exposed as an optional creation switch because the scoped rule is part of what makes the object an Agent Workspace. New workspaces default to read-only access unless writes are explicitly selected.

Storage-level MCP exposure remains independent. A workspace may be prepared while its storage is not exposed, but an MCP client cannot see it until that storage is exposed. The managed workspace rule remains the authority for the allowed prefix and access mode once exposure is enabled.

For Local Filesystem storage, the configured storage root must be an absolute host path. Shell notation such as `$HOME/...` is not expanded by the desktop field. Edit and validate an invalid storage configuration before creating a workspace. Workspace paths such as `/agent-workspaces/example` are then interpreted relative to that configured storage root.

Creation refuses invalid workspace roots, path traversal, overlapping workspace roots, disabled or read-only storages, and unexpected existing content unless the adoption flow explicitly accepts it. A failed multi-step creation attempts to remove files and directories it created and restore prior policy state.

## Optional starter notes

The legacy coding, research, writing, and data-analysis plans create visible Markdown starter files under `memory/`. They remain supported for existing workspaces and are available as optional starter content for new workspaces. They do not select an agent runtime or grant additional permissions.

A plain workspace is sufficient for Agent Tasks. Agent Tasks create their own `TASK.md`, `task-manifest.json`, `inputs/`, `outputs/`, and publication receipts beneath `tasks/<task-id>`.

Checkpoint manifests are stored under `.infimount/checkpoints/`. A checkpoint records only managed starter-note files and can restore those files. A plain workspace with no managed starter notes has nothing useful to checkpoint. Backend permissions and capabilities still apply.

## Policy behavior

Workspace rules are segment-aware path-prefix rules. Denied prefixes override grants. Changing a workspace access profile updates only its managed rule and preserves unrelated storage policy rules. Deleting a workspace removes its managed rule; it does not delete the workspace's storage files unless the user explicitly chooses the separate delete-files action.

The workspace-managed rule on a storage cannot be edited, re-prefixed, or removed through the storage policy editor; it is enforced by the bound workspace. A generic storage policy update that alters workspace-sourced rules or adds a manual rule under a workspace root is rejected.

Multiple workspaces may share a storage when their normalized roots do not overlap and their names are unique within that storage.

## Namespace binding

Each workspace is bound to the namespace identity of the storage it references, captured as a fingerprint of the backend, account authority, container, and canonical root. The storage namespace is verified whenever workspace access or policy rules change.

- Editing a storage so its namespace changes while workspaces are bound is rejected (`ERR_STORAGE_NAMESPACE_IN_USE`); delete or recreate the workspaces first.
- Removing a storage that still has bound workspaces is rejected (`ERR_STORAGE_HAS_WORKSPACES`).
- Changing storage credentials while workspaces are bound requires explicit confirmation and validation of the updated storage before it is committed (`ERR_CONFIRMATION_REQUIRED` otherwise). After the change, verify each affected workspace still maps to the same account and namespace.
- Pre-v0.8 browser-local workspaces are not imported. Recreate them in Agent Workspaces after upgrading.

## Agent Tasks

Agent Tasks v1 require a read-write Agent Workspace backed by Local Filesystem storage. The workspace storage must be MCP-exposed before Codex handoff, but the source storage selected for an Agent Task does not need MCP exposure and task preparation never grants it.

This separation is intentional: the Agent Workspace is the bounded working area, while the task source remains governed by its existing storage policy.

## Limits

- Workspace metadata and policy changes coordinate local registry files; they cannot provide a distributed transaction with a remote storage service.
- Recovery after an interrupted remote write depends on backend behavior.
- Workspace activity combines available local product events and MCP audit entries; it is not a complete provider-side access log.
