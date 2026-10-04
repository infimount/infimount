# Infimount 0.8.1-rc.14: final compatibility fixes from the real pilot

Release: https://github.com/infimount/infimount/releases/tag/v0.8.1-rc.14

Infimount 0.8.1-rc.14 carries the rc.13 release/supply-chain boundary and fixes the two runtime compatibility defects exposed by the real rc.13 Agent Task pilot.

## Why rc.14

The real rc.13 pilot completed all three deterministic workload validators:

- coding output applied cleanly and passed source tests;
- document synthesis retained the required source facts;
- data-analysis metrics matched the independent expected values;
- source digests remained unchanged;
- stale-preview rejection, fail-on-conflict, no-overwrite, destination verification, and publication receipts were observed;
- the quality-pending evidence bundle passed the real evidence validator.

That run also found two compatibility defects that stable must not inherit.

### Legacy Local Filesystem `~` source roots

Browsing already accepted legacy Local Filesystem roots such as `~` and `~/...`, but Agent Task namespace/confinement resolution required the configured root to already be absolute. A storage could therefore browse successfully and then fail during Agent Task preparation.

PR #130 reuses the existing Local Filesystem home-alias expansion in namespace/confinement canonicalization. Legacy `~` roots now resolve consistently across operator construction, namespace identity, MCP confinement, and Agent Task preparation. Shell-variable forms such as `$HOME/...` remain invalid.

The namespace fingerprint logic preserves the existing public-config representation for already-canonical absolute roots while making a legacy alias equivalent to the same root after expansion.

### Current Codex MCP routing

The first-class Agent Task Codex launcher forced `features.code_mode_host=false`. Current Codex clients route MCP/tool traffic through the code-mode host process even when code mode itself remains disabled, so the handoff could fail before using the task-scoped MCP server.

rc.14 enables only:

`features.code_mode_host=true`

The security boundary remains intentionally narrow:

- `features.code_mode=false`;
- shell tool disabled;
- unified exec disabled;
- shell snapshot disabled;
- exec/request-permission tools disabled;
- hooks disabled;
- multi-agent disabled;
- browser/computer use disabled;
- plugins/apps disabled;
- project docs and skills disabled;
- neutral temporary working directory;
- read-only Codex sandbox;
- task-unique required Infimount MCP server with the bounded Agent Task allowlist.

## Automated qualification

PR #130 passed:

- CI;
- Integration Tests;
- Coverage;
- Dependency Audit;
- Repo Lint;
- Release Rehearsal;
- the real desktop Release Pilot.

The Release Pilot explicitly restored the Local Filesystem source root to `~` before Agent Task preparation and passed with:

- `agent_task_legacy_home_source=yes`;
- `legacy_home_agent_task_prepare=yes`;
- task-scoped sidecar independent-gate checks;
- task-scoped tool-surface checks;
- task-scoped confinement checks.

The Codex handoff tests also passed while asserting the host router is enabled and code mode plus ambient execution surfaces remain disabled.

## Required rc.14 release validation

Before tagging, the exact rc.14 release-preparation head must pass:

- CI;
- Integration Tests;
- Coverage;
- Dependency Audit;
- Repo Lint;
- Release Rehearsal;
- Installer Upgrade Harness where applicable;
- the real desktop Release Pilot.

After tagging, canonical Release and automatic Post Release Validation must pass, including:

- all deterministic Release Gates;
- Linux, macOS, and Windows package builds;
- updater signing/key correspondence;
- v0.8.0-to-rc.14 Linux installer-over-install validation;
- checksums, SBOM, updater metadata, provenance, install scripts, and package validation;
- draft re-download;
- publication;
- public re-download;
- published-package upgrade validation.

## Focused real regression after publication

Stable promotion does not require repeating unrelated rc.13 data/document coverage. It does require one real packaged rc.14 coding run that combines both changed paths:

1. use a Local Filesystem source storage whose configured root remains `~`;
2. browse the pilot coding source successfully;
3. prepare an Agent Task without manually rewriting the storage root;
4. launch through Infimount's first-class current Codex handoff with no manual `code_mode_host` override;
5. produce `outputs/review.md` and `outputs/patch.diff`;
6. pass the existing coding workload validator;
7. confirm source bytes and source MCP exposure remain unchanged;
8. confirm the scoped MCP/sandbox boundary remains intact.

The rc.13 three-workload results remain product-value evidence. Final human usefulness acceptance remains explicit and separate from deterministic CI.

## Signing boundary

Updater signing remains mandatory. macOS platform signing/notarization and Windows Authenticode are used when their complete credentials are configured; otherwise the release records those platform packages as unsigned. rc.13 qualified the explicit unsigned-platform path.

## Known boundaries

- v0.8.0 remains the stable public release until v0.8.1 promotion completes.
- rc.13 remains immutable evidence of the release infrastructure and the full three-workload pilot that exposed the two rc.14 fixes.
- Agent Tasks v1 require a read-write Local Filesystem Agent Workspace for writable task outputs.
- Agent Task publication intentionally has no overwrite mode.
- Storage browsing alone does not expose storage to agents.
- The documented local-filesystem TOCTOU boundary remains part of the current local-process trust model.
