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

Multiple workspaces may share a storage when their normalized roots do not overlap and their names are unique within that storage.

## Limits

- Workspace metadata and policy changes coordinate local registry files; they cannot provide a distributed transaction with a remote storage service.
- Recovery after an interrupted remote write depends on backend behavior.
- Workspace activity combines available local product events and MCP audit entries; it is not a complete provider-side access log.
