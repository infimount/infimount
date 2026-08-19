# Agent Workspaces

Agent Workspaces create a project-shaped area on an existing Infimount storage and attach an MCP path rule to that area.

## Create a workspace

1. Add and validate a storage.
2. Open **Agent Workspaces** and choose a template.
3. Choose the storage, workspace name, root, and access profile.
4. Review the generated files before creating the workspace.

The desktop generates the workspace ID, validates the normalized root, writes the template files through OpenDAL, persists the workspace registry, and creates a policy rule named `workspace:<id>`. New workspaces default to read-only access unless write access is explicitly selected. A storage still must be enabled and exposed to MCP before an MCP client can see it.

Creation refuses storage roots, path traversal, overlapping workspace roots, disabled storages, and unexpected existing content unless the adoption flow explicitly accepts it. A failed multi-step creation attempts to remove files and directories it created and restore prior policy state.

## Templates and memory

The coding, research, and data-analysis templates create visible Markdown files under `memory/`. These are ordinary files in the selected backend, not hidden prompts. Review them before exposing the workspace to an agent.

Checkpoint manifests are stored under `.infimount/checkpoints/`. A checkpoint records workspace memory files and can restore those files. Backend permissions and capabilities still apply.

## Policy behavior

Workspace rules are segment-aware path-prefix rules. Denied prefixes override grants. Changing a workspace access profile updates only its managed rule and preserves unrelated storage policy rules. Deleting a workspace removes its managed rule; it does not delete the workspace's storage files.

The workspace-managed rule on a storage cannot be edited, re-prefixed, or removed through the storage policy editor; it is enforced by the bound workspace. A generic storage policy update that alters workspace-sourced rules or adds a manual rule under a workspace root is rejected.

Multiple workspaces may share a storage when their normalized roots do not overlap and their names are unique within that storage.

## Namespace binding

Each workspace is bound to the namespace identity of the storage it references, captured as a fingerprint of the backend, account authority, container, and canonical root. The storage namespace is verified whenever workspace access or policy rules change.

- Editing a storage so its namespace changes while workspaces are bound is rejected (`ERR_STORAGE_NAMESPACE_IN_USE`); delete or recreate the workspaces first.
- Removing a storage that still has bound workspaces is rejected (`ERR_STORAGE_HAS_WORKSPACES`).
- Changing storage credentials while workspaces are bound requires explicit confirmation and validation of the updated storage before it is committed (`ERR_CONFIRMATION_REQUIRED` otherwise). After the change, verify each affected workspace still maps to the same account and namespace.
- Pre-v0.8 browser-local workspaces are not imported. Recreate them in Agent Workspaces after upgrading.

## Limits

- Workspace metadata and policy changes coordinate local registry files; they cannot provide a distributed transaction with a remote storage service.
- Recovery after an interrupted remote write depends on backend behavior.
- Workspace activity combines available local product events and MCP audit entries; it is not a complete provider-side access log.
