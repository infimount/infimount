# Agent Tasks implementation checkpoint

This temporary development note exists only to make the feature branch produce a reviewable checkpoint while v0.8.1 Agent Tasks is being assembled.

Implemented on this branch:

- strict task package format under `tasks/<uuid>`;
- local-only read/write Agent Workspace requirement;
- source selection preflight with bounded file/byte limits;
- source-copy preparation through existing OpenDAL transfer primitives;
- prepared-input SHA-256 capture;
- hidden staging and rename commit for task packages;
- task discovery and output hashing;
- reviewed-output publication via a private local snapshot plus destination atomic create-if-absent;
- per-output published/conflict/stale/failed results;
- desktop Agent Tasks hub for preparation, review, preview, and publication.

The task manifest and `TASK.md` remain non-authoritative. Existing workspace/MCP policy is the authorization boundary.

This file should be removed or folded into `docs/agent-tasks.md` before the release candidate.
