# Infimount 0.8.1-rc.9: prove release-critical Agent Access behavior before tagging

Release: not published yet.

Infimount 0.8.1-rc.9 carries the same Agent Access product model intended for rc.8 and fixes the release-validation gap that allowed a deterministic packaged-sidecar activation failure to appear only after the rc.8 tag had already been created.

## Why rc.9

`v0.8.1-rc.8` was tagged at the intended validated main commit, but its canonical Release workflow did not reach packaging or publication.

The Release Gate failed only in the packaged stdio activation proof. PR #110 had correctly changed ordinary `infimount_mcp serve` to fail closed unless persisted general Agent Access is enabled. The activation smoke fixture, however, still persisted `McpSettings::default()`, where `enabled=false`.

The packaged sidecar therefore exited before MCP initialization and the test surfaced:

```text
ERR_MCP_HANDSHAKE_FAILED
```

Every other deterministic rc.8 Release Gate passed, including signing policy, frontend checks, Playwright UI, Rust, Storage Simulator, release consistency, and zero-manual release policy. rc.8 was not published and is not a real-pilot candidate.

PR #112 fixes the mismatch without weakening the production Agent Access gate.

## Activation proof now models the real guided state

The packaged-sidecar activation fixture now explicitly persists general Agent Access as enabled before launching ordinary stdio `serve`, matching the state created by the guided Agent Access path.

The smoke also asserts that persisted readiness invariant before starting the packaged sidecar. A future fixture drift therefore fails at the actual missing precondition instead of collapsing into an opaque handshake error.

Activation smoke failures now retain test output and Rust backtraces through `--nocapture` and `RUST_BACKTRACE=1`.

The production behavior is unchanged:

- ordinary `infimount_mcp serve` still fails closed when general Agent Access is disabled;
- the guided path still requires the exact managed workspace policy boundary;
- `serve-agent-task` remains separately scoped to its Agent Task/workspace contract.

## Release-critical deterministic checks now run before tagging

The rc.8 failure exposed a process defect: mandatory pre-tag workflows did not execute every deterministic state smoke used by the canonical Release Gate.

PR #112 closes that gap.

Pre-tag Integration Tests now run, in the same Desktop Smoke job:

- the normal desktop launch/migration smoke;
- packaged Agent Access activation smoke;
- secret-migration smoke;
- backup/restore smoke.

Pre-tag Desktop Smoke also uses the hardened CI APT installer used by Release, preserving the Chrome-repository isolation added after the rc.2 runner incident.

Repo Lint now additionally exercises the remaining cheap deterministic release-policy fixtures before a tag exists:

- upgrade fixtures;
- installer-script smoke;
- packaged-sidecar checksum fixture validation.

This changes the release invariant: deterministic repository behavior covered by the canonical Release Gate must be proven on the exact release-preparation PR head before the candidate tag is created. Tag-triggered Release may still depend on signing credentials and external platform infrastructure, but it should no longer be the first place these repository invariants are exercised.

## Agent Access product model retained from rc.8

rc.9 does not introduce another Agent Access redesign. It carries forward the product behavior implemented by PR #110:

- storage browsing is a valid complete onboarding path through **Browse storage only**;
- storage-only completion does not expose storage to MCP or leave a floating activation reminder;
- Agent Access is a first-class product surface;
- **Connect agent** from Agent Workspaces retains the selected workspace;
- the normal flow is **Storage → Workspace → Agent Access → Connect Client → Safety Probe**;
- stdio is client-launched/on-demand and has no background-server Start control;
- HTTP has explicit stopped/running state and Start/Stop controls;
- shown client configuration follows the selected transport;
- verification is described as a packaged sidecar/workspace-policy safety probe, not HTTP endpoint verification;
- guided readiness mirrors the backend fail-closed workspace-policy contract;
- broader tools, broader/manual storage grants, or non-loopback HTTP require Advanced MCP review;
- Advanced MCP stdio saves preserve the explicit general Agent Access gate;
- dismissible warnings do not bypass their underlying enforcement.

## Guided readiness remains least privilege

A workspace is considered prepared only when the effective storage policy matches the managed workspace contract, including:

- exact managed workspace rule ID;
- exact workspace rule source;
- exact normalized workspace prefix;
- access matching the workspace profile;
- `default_access=none`;
- no broader positive manual grants;
- no read/write workspace grant on read-only storage.

The prepare path performs a read-only preflight and repeats the relevant checks under the configuration lock before exposing the backing storage.

## File-browser and legacy-root corrections retained

rc.9 retains the previous candidate corrections:

- legacy Local Filesystem `~` / `~/...` roots are canonically normalized before first workspace namespace binding;
- continuation is scroll-driven instead of a visible manual **Load more** workflow;
- revision-stale cursors refresh the current directory;
- genuine paging failures stop automatic retries and expose retry state;
- stale responses remain guarded after navigation or storage changes.

## Agent Task safety model retained

rc.9 does not broaden Agent Task preparation or publication semantics:

- task preparation copies only explicitly selected source files;
- source bytes are not mutated by preparation;
- source MCP exposure is not expanded;
- publication starts with nothing selected;
- only `fail` and `rename` conflict handling are exposed;
- overwrite is unavailable;
- publication uses create-only writes;
- stale approved previews are rejected;
- committed destination bytes are verified;
- successful publication creates a unique publication receipt;
- partial multi-file failure reports cleanup-required state instead of claiming unsafe rollback.

Agent Tasks v1 remain limited to a read-write Local Filesystem Agent Workspace even though Agent Workspaces themselves can cover a broader set of writable OpenDAL backends.

## Product validation

Once published and post-release validation passes, `v0.8.1-rc.9` supersedes rc.8 as the candidate for the resumed real pilot.

The pilot starts from representative v0.8.0 local state and installs rc.9 over it. It must cover:

- configuration, storage-registry, and Agent Workspace retention;
- legacy `~` normalization without manual storage editing;
- storage-only onboarding completion without Agent Access exposure;
- the first-class Agent Access and **Connect agent** flows;
- stdio and HTTP runtime semantics;
- exact least-privilege workspace readiness;
- ordinary Agent Access disabled-state enforcement and independent Agent Task serving;
- fail-closed routing to Advanced MCP for broader authority;
- >200-entry scroll continuation and stale-cursor recovery;
- one real coding Agent Task;
- one real document Agent Task;
- one real data-analysis Agent Task;
- Codex handoff through Infimount MCP;
- unchanged source-selection bytes and source MCP exposure;
- fail-on-conflict rejection;
- stale-preview rejection;
- no overwrite publication path;
- explicit review, approval, destination verification, and publication receipt evidence.

See `docs/agent-tasks-pilot.md` for the complete protocol. No completed real rc.9 pilot is claimed by this release preparation.

## Release validation

Before creating the `v0.8.1-rc.9` tag, the exact final release-preparation PR head must pass all six mandatory workflows:

- CI;
- Integration Tests, including the release-critical state smokes;
- Release Rehearsal;
- Coverage;
- Dependency Audit;
- Repo Lint, including deterministic release-policy fixtures.

Only after those exact-head checks pass may the release-preparation PR merge and the annotated tag be created on the resulting main merge commit.

After tagging, the canonical Release workflow must pass the complete chain, including:

- signing-policy checks;
- frontend/UI/Rust/desktop state/storage/consistency/policy gates;
- Linux, macOS, and Windows packaging;
- updater-signature verification;
- checksums, SBOM, provenance, and installer smoke checks;
- draft-release asset validation;
- publication and published-release asset re-download validation.

The automatic downstream `Post Release Validation` workflow must also pass before rc.9 is accepted for the resumed real pilot.

## Known boundaries

- v0.8.0 remains the stable public release while rc.9 is evaluated.
- rc.8 remains an immutable failed candidate tag and is not moved or reused.
- Agent Tasks v1 require a read-write Local Filesystem Agent Workspace.
- Storage browsing alone does not expose storage to agents.
- Agent Workspace creation alone does not imply that normal Agent Access is active.
- The built-in MCP HTTP transport remains loopback-oriented in the guided path; explicitly retained broader HTTP configurations require Advanced MCP review and existing authentication/start protections.
- Advanced MCP configurations with broader tools remain supported, but the guided Agent Access path intentionally refuses to activate a new workspace into them without explicit review.
- Local filesystem symlink/reparse-point defenses retain their documented check-then-OpenDAL TOCTOU limitation.
- Agent Task publication intentionally has no overwrite mode.
