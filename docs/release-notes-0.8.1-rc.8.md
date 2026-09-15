# Infimount 0.8.1-rc.8: Agent Access as a first-class product boundary

Release: not published yet.

Infimount 0.8.1-rc.8 is the eighth release candidate for Agent Tasks. It keeps the rc.7 Agent Workspace, file-browser, release-pipeline, and publication-safety behavior while replacing the remaining activation/MCP-admin coupling with a first-class Agent Access experience and a persisted general Agent Access gate.

## Why rc.8

`v0.8.1-rc.7` was published successfully. Its canonical Release workflow and automatic downstream `Post Release Validation` both passed.

The resumed real product validation then showed that the rc.7 reminder fix addressed a symptom, not the underlying product model. Storage browsing, workspace scoping, agent handoff, and MCP administration were still too tightly coupled for the normal user path. In particular:

- storage-only use could still be framed as incomplete activation even though browsing storage is a valid complete use case;
- Agent Access was discoverable mainly through MCP administration rather than as a first-class product surface;
- stdio and HTTP were not presented with sufficiently distinct runtime semantics;
- the normal verification action could be read as endpoint verification even though it is a packaged sidecar/workspace-policy safety probe;
- the persisted general Agent Access setting was not enforced directly by ordinary `infimount_mcp serve`;
- the guided frontend readiness model needed to mirror the backend fail-closed workspace-policy checks exactly.

PR #110 fixes that model. Because it changes normal user flow and the runtime security gate after rc.7, a new rc.8 candidate is required before stable promotion.

## Storage browsing is complete on its own

First-run onboarding now treats storage browsing as a complete product path.

A user can choose **Browse storage only** and finish onboarding without:

- exposing a storage to an agent;
- creating an Agent Workspace;
- enabling MCP;
- being left in a permanent activation-incomplete state;
- receiving the old floating activation reminder.

Agent Access remains available later from its dedicated product surface.

## Agent Access is first class

The normal agent path is now explicit:

**Storage → Workspace → Agent Access → Connect Client → Safety Probe**

Agent Access is available directly from the main product navigation and from Agent Workspaces through **Connect agent**. The selected workspace is carried into Agent Access so the handoff does not lose the user's scope decision.

The normal surface shows the selected workspace boundary, current readiness, transport behavior, client configuration, and safety probe without requiring the user to reason about the full Advanced MCP administration model.

## Clear transport semantics

### stdio

stdio is presented as on demand:

- the MCP process is launched by the client when needed;
- no background server is required;
- there is no Start server button;
- the stdio client configuration is shown when stdio is selected.

### HTTP

HTTP remains a persistent runtime:

- stopped and running are distinct states;
- Start and Stop controls are explicit;
- the HTTP client configuration is shown when HTTP is selected;
- non-loopback HTTP retains its authentication and confirmation requirements.

A disabled first-time guided setup still chooses local stdio instead of activating a previously drafted HTTP listener.

## Safety probe wording is precise

The normal verification action is now described as a **safety probe**.

It validates the packaged MCP sidecar and effective workspace-policy boundary. It does not claim to prove that a selected HTTP endpoint is reachable or authenticated end to end.

This distinction is now represented in both the UI and automated coverage.

## Persisted general Agent Access gate

Ordinary `infimount_mcp serve` now enforces the persisted general Agent Access setting before exposing tools.

If general Agent Access is disabled, the ordinary server fails closed instead of relying only on the desktop UI to avoid launching it.

The Agent Task server remains separately scoped:

- `serve-agent-task` does not inherit the general Agent Access gate;
- Agent Task safety remains governed by its dedicated workspace/task path;
- disabling normal Agent Access therefore does not silently redefine Agent Task semantics.

## Guided readiness mirrors the backend boundary

A workspace is considered prepared only when its effective storage policy matches the managed workspace contract. The guided UI now mirrors the backend checks for:

- exact managed workspace rule ID;
- exact managed rule source/workspace identity;
- exact workspace prefix;
- access level matching the workspace profile;
- `default_access=none`;
- no broader positive manual grants that would exceed the guided boundary;
- no read/write workspace grant on read-only storage.

The prepare path still performs a read-only preflight first, then repeats the relevant checks under the configuration lock before exposing the backing storage.

An already-active configuration with additional global tools, broader/manual storage grants, or a non-loopback HTTP listener continues to fail closed into Advanced MCP review rather than silently inheriting broader authority.

## Advanced MCP remains available without owning the normal path

Advanced MCP settings remain the escape hatch for explicit administrative configurations.

The rc.8 changes preserve important advanced behavior while removing it from the ordinary setup path:

- stdio saves preserve the explicit general Agent Access gate instead of silently changing it;
- HTTP start/stop remains available;
- non-loopback warnings remain dismissible without bypassing auth/start enforcement;
- touched persistent warning bars can be dismissed;
- readiness/runtime conditions are expressed as status rather than sticky notification overlays.

## Dependency security update

The rc.8 candidate also carries the dependency correction merged with PR #110:

- `rustls` advances from 0.23.43 to 0.23.45;
- `rustls-webpki`, `aws-lc-rs`, and `aws-lc-sys` advance with it;
- the exact PR head passes the repository Dependency Audit, closing the RustSec blocker that appeared during rc.8 preparation.

## rc.6 and rc.7 corrections retained where still applicable

rc.8 retains the file-browser correction introduced for rc.6:

- continuation is scroll-driven rather than a visible manual **Load more** workflow;
- revision-stale cursors refresh the current directory instead of surfacing a generic continuation error;
- genuine paging failures stop automatic retries and expose an explicit retry state;
- stale responses remain guarded after navigation or storage changes.

The rc.7 session-only activation reminder itself is intentionally superseded rather than retained. The new product model removes the floating reminder because storage-only onboarding can now complete legitimately without Agent Access.

## Agent Workspace and Agent Task capability boundary

Agent Workspace creation remains a storage-scoped MCP abstraction over supported writable OpenDAL backends.

Agent Tasks v1 remain narrower:

- the Agent Task workspace must be read-write;
- Agent Tasks currently require a Local Filesystem workspace because the task pipeline relies on stronger local filesystem and atomic-rename assumptions;
- task source storage remains independent and is never newly exposed by preparation;
- remote/object-backed Agent Workspaces do not imply Agent Task support on those backends.

rc.8 does not expand that capability boundary.

## Publication safety model retained

rc.8 does not broaden Agent Task publication semantics:

- task preparation copies only explicitly selected source files and does not mutate the source;
- source MCP exposure is not expanded by task preparation;
- output review records current size and SHA-256 fingerprints;
- publication begins with nothing selected;
- publication supports only `fail` and `rename` conflict handling;
- overwrite is not exposed;
- publication uses create-only writes;
- stale approved previews are rejected;
- committed destination bytes are verified;
- successful publication creates a unique `publish-receipt-<publication-id>.json` receipt;
- partial multi-file failure surfaces cleanup-required state instead of claiming unsafe rollback.

## Product validation

Once published, `v0.8.1-rc.8` supersedes rc.7 as the real-pilot candidate.

The resumed pilot must cover:

- a representative v0.8.0 → rc.8 installer-over-install compatibility exercise with pre-existing configuration, storage registrations, and at least one Agent Workspace;
- legacy `~` Local Filesystem root normalization without manual storage editing;
- successful storage-only onboarding completion with no Agent Access exposure and no persistent activation reminder;
- the first-class Agent Access surface and the **Connect agent** handoff from Agent Workspaces;
- the normal **Storage → Workspace → Agent Access → Connect Client → Safety Probe** flow;
- least-privilege tool selection and per-workspace readiness;
- stdio shown as client-launched/on-demand with no server Start control;
- HTTP explicit stopped/running Start/Stop behavior;
- transport-appropriate client configuration;
- safety-probe wording that does not claim HTTP endpoint verification;
- ordinary Agent Access disabled-state enforcement and independent Agent Task serving semantics;
- fail-closed routing to Advanced MCP for broader tools, broader/manual grants, and non-loopback HTTP;
- a >200-entry disposable directory proving scroll-driven continuation and stale-cursor recovery;
- one real coding Agent Task;
- one real document Agent Task;
- one real data-analysis Agent Task;
- Codex handoff through Infimount MCP;
- unchanged source bytes and unchanged source MCP exposure;
- fail-on-conflict rejection;
- stale-preview rejection;
- absence of overwrite publication;
- explicit review, approval, destination verification, and publication receipt evidence.

See `docs/agent-tasks-pilot.md` for the complete protocol. No completed real rc.8 pilot is claimed by this release preparation.

## Release validation

Before creating the `v0.8.1-rc.8` tag, the exact final release-preparation PR head must pass all six pre-tag workflows:

- CI;
- Integration Tests;
- Release Rehearsal;
- Coverage;
- Dependency Audit;
- Repo Lint.

After the tag is created, the canonical Release workflow must pass the tag-triggered chain, including:

- signing-policy checks;
- Linux, macOS, and Windows packaging;
- updater-signature verification;
- checksums, SBOM, provenance, and installer smoke checks;
- draft-release re-download validation;
- publication and published-release re-download validation.

The automatic downstream `Post Release Validation` workflow must also pass before rc.8 is accepted as the candidate for the resumed real pilot.

## Known boundaries

- v0.8.0 remains the stable public release while rc.8 is evaluated.
- Agent Tasks v1 require a read-write Local Filesystem Agent Workspace.
- Storage browsing alone does not expose storage to agents.
- Agent Workspace creation alone does not imply that normal Agent Access is active.
- The built-in MCP HTTP transport remains loopback-oriented in the normal path, while explicitly retained broader HTTP configurations require Advanced MCP review and their existing auth/start protections.
- Advanced MCP configurations with broader tools remain supported, but the guided Agent Access path intentionally refuses to activate a new workspace into them without explicit review.
- Local filesystem symlink/reparse-point defenses retain their documented check-then-OpenDAL TOCTOU limitation.
- Agent Task publication intentionally has no overwrite mode.
