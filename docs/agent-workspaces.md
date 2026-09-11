# Agent Workspaces

Agent Workspaces define a storage-scoped MCP boundary for agents and Agent Tasks. The workspace identity is the selected storage, its managed workspace folder, and the access profile. Agent-specific templates are not part of the authorization model or new-workspace flow.

## Guided flow

The normal end-user path is:

1. **Storage**: add or choose a storage for browsing.
2. **Workspace**: create a scoped workspace on that storage.
3. **Agent Access**: explicitly prepare the selected workspace for MCP access.
4. **Connect Client**: review and install or copy the MCP client integration.
5. **Verify**: run the packaged safety probe before setup can finish.

Adding a storage does not expose it to agents. Creating a workspace does not automatically expose the backing storage either. The full **MCP Settings** surface is an advanced administration path, not a required onboarding step.

## Create a workspace

1. Add an enabled writable storage with a valid backend configuration.
2. Open **Agent Workspaces**.
3. Choose the storage and enter a workspace name.
4. Leave agent writes off for read-only access, or explicitly enable writes when the workspace must host Agent Task outputs.
5. Review the derived workspace location and create the workspace.

Workspace creation is backend-agnostic at the product layer. Any enabled writable storage may be selected when its OpenDAL operator supports the operations required to create and use the workspace. Backend permissions and capabilities still apply, so a provider that cannot perform the required writes will fail validation or creation rather than being treated as a special Local Filesystem-only workflow.

The desktop derives the workspace folder from the name under `/agent-workspaces/<name>` and shows that storage-relative location before creation. This is a path inside the selected storage, not another host filesystem root to configure.

New workspaces are plain scoped folders. Normal desktop creation always creates the workspace-managed MCP path rule. The policy choice is not exposed as an optional creation switch because the scoped rule is part of what makes the object an Agent Workspace. New workspaces default to read-only access unless writes are explicitly selected.

Storage-level MCP exposure remains independent. A workspace may be prepared while its storage is not exposed, but an MCP client cannot see it until the user explicitly prepares agent access. The managed workspace rule remains the authority for the allowed prefix and access mode once exposure is enabled.

For Local Filesystem storage, a literal absolute root remains the canonical persisted identity. Legacy home aliases already accepted by normal browsing, such as `~` and `~/projects`, are accepted by workspace setup and are normalized once to the current user's canonical absolute home path before the first workspace binds that storage namespace. Shell-variable forms such as `$HOME/...` or `${HOME}/...` are not expanded and must be edited to `~` or an absolute folder path. A legacy `~` root is never rewritten after workspaces are already bound to that storage.

Creation refuses invalid workspace roots, path traversal, overlapping workspace roots, disabled or read-only storages, and unexpected existing content unless the adoption flow explicitly accepts it. A failed multi-step creation attempts to remove files and directories it created and restore prior policy state.

## Prepare agent access

Agent access is an explicit step for the selected workspace. The guided path performs a read-only preflight before changing MCP runtime settings. The preflight verifies the workspace schema, storage namespace binding, workspace-managed path rule, access profile, and storage state.

If the preflight succeeds, Infimount enables only the minimum MCP tools required by that workspace profile. Read-only workspaces receive the read tools. Read-write workspaces add only `mkdir` and `write_file` to that minimum set. The storage is exposed only after the runtime update succeeds, and the policy and namespace checks are repeated under the configuration lock before exposure is committed.

The guided path does not silently rewrite broader policy. If the storage has broad default MCP access or additional manual grants, setup stops and asks the user to review **Advanced MCP settings** instead.

Readiness is workspace-specific during guided setup. Preparing one workspace does not mark another workspace ready, including when both workspaces share the same storage. Switching the selected workspace requires that workspace to pass the preparation step before onboarding can continue.

## Legacy starter-note workspaces

Earlier workspace versions could create coding, research, writing, data-analysis, or other template-specific Markdown files under `memory/`. Existing current-schema workspaces that already contain those managed starter files remain readable and keep their checkpoint behavior for compatibility.

New workspace creation does not ask for an agent type or starter template. A plain workspace is sufficient for Agent Tasks because each Agent Task creates its own `TASK.md`, `task-manifest.json`, `inputs/`, `outputs/`, and publication receipts beneath `tasks/<task-id>`.

Checkpoint manifests for existing templated workspaces are stored under `.infimount/checkpoints/`. A checkpoint records only those managed starter-note files and can restore those files. Plain workspaces have no managed starter notes to checkpoint. Backend permissions and capabilities still apply.

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

Agent Tasks v1 still require a read-write Agent Workspace backed by Local Filesystem storage. The workspace storage must be MCP-exposed before Codex handoff, but the source storage selected for an Agent Task does not need MCP exposure and task preparation never grants it.

This is a narrower capability boundary than Agent Workspace creation. Remote/object-backed workspaces can be created and used as scoped MCP workspaces when their operator supports the required operations, but this release does not claim that the Agent Tasks pipeline itself supports every backend.

This separation is intentional: the Agent Workspace is the bounded working area, while the task source remains governed by its existing storage policy.

## Limits

- Workspace metadata and policy changes coordinate local registry files; they cannot provide a distributed transaction with a remote storage service.
- Recovery after an interrupted remote write depends on backend behavior.
- Workspace activity combines available local product events and MCP audit entries; it is not a complete provider-side access log.
