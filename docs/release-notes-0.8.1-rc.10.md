# Infimount 0.8.1-rc.10: isolate release publication reruns

Release: not published yet.

Infimount 0.8.1-rc.10 carries the same product behavior intended for rc.9 and fixes the release-publication boundary exposed only after rc.9 had already passed every release gate and all three platform builds.

## Why rc.10

`v0.8.1-rc.9` was tagged at the intended validated main commit. Its canonical Release workflow passed the signing, frontend, Playwright UI, Rust, desktop state, storage simulator, release consistency, and zero-manual policy gates. Linux, macOS, and Windows packaging also completed successfully.

The remaining failure was in publication revalidation on a rerun, not in Infimount product behavior or platform packaging.

The mandatory SBOM step used `anchore/sbom-action@v0`. That action defaults to uploading both a workflow artifact and, for a tag build, a release asset. On the rc.9 rerun it attached its raw pre-augmentation SBOM as:

```text
infimount-create-release.spdx.json
```

The Infimount release publisher intentionally owns a different curated SBOM, `SBOM.spdx.json`, after bundled-sidecar augmentation. `SHA256SUMS.txt` correctly covered that curated payload and did not include the action-owned raw SBOM. When the existing draft was re-downloaded, strict checksum validation therefore rejected the unexpected extra asset rather than silently accepting an untracked file.

PR #114 fixes the producer/publisher boundary instead of weakening checksum validation. rc.9 was not published and is not a real-pilot candidate.

## One publisher owns the curated release asset set

The canonical Release workflow now makes the ownership boundary explicit:

- the publisher downloads only platform build artifacts matching `*-artifacts`;
- `anchore/sbom-action` runs with `upload-artifact: false`;
- `anchore/sbom-action` runs with `upload-release-assets: false`;
- the generated SBOM is written directly to `release-assets/SBOM.spdx.json`;
- the bundled Linux, macOS, and Windows MCP sidecars are added to that SBOM before validation;
- checksums are generated only after the final curated payload is complete;
- draft and published release assets are still re-downloaded and validated strictly.

This makes a release rerun idempotent with respect to SBOM ownership: the SBOM generator produces bytes, while the release publisher alone decides which bytes become GitHub Release assets.

## The rerun invariant is enforced before tagging

The zero-manual release-policy gate now requires the release workflow to retain all three isolation properties:

- `pattern: '*-artifacts'` for the publisher download;
- `upload-artifact: false` for SBOM generation;
- `upload-release-assets: false` for SBOM generation.

The policy also rejects `upload-release-assets: true` if it is reintroduced. This turns the rc.9 publication failure into a deterministic repository invariant checked before a future release tag exists.

The checksum and asset validators remain fail closed. No exception was added for unexpected draft assets.

## Agent Access behavior remains unchanged

rc.10 does not add another Agent Access redesign. It retains the product model introduced before rc.8 and proven through the subsequent deterministic gates:

- **Browse storage only** is a legitimate completed onboarding path;
- storage-only onboarding does not expose storage to MCP or leave an activation reminder;
- Agent Access is a first-class product surface;
- **Connect agent** from Agent Workspaces retains the selected workspace;
- the normal path is **Storage → Workspace → Agent Access → Connect Client → Safety Probe**;
- stdio remains client-launched/on-demand and has no background-server Start control;
- HTTP keeps explicit stopped/running state and explicit Start/Stop controls;
- client configuration follows the selected transport;
- verification is a packaged-sidecar/workspace-policy safety probe, not a claim that an arbitrary HTTP endpoint was verified;
- guided readiness mirrors the backend least-privilege workspace-policy contract;
- broader tools, broader/manual storage grants, or non-loopback HTTP require Advanced MCP review;
- ordinary `infimount_mcp serve` still fails closed when general Agent Access is disabled;
- `serve-agent-task` remains independently scoped to its Agent Task/workspace path;
- Advanced MCP stdio saves preserve the explicit general Agent Access gate.

The activation fixture correction from PR #112 is retained. The production disabled-state gate was never weakened.

## Existing workflow corrections remain in scope

rc.10 also retains the candidate fixes already accumulated during the real validation cycle:

- legacy Local Filesystem `~` and `~/...` roots normalize canonically before first Agent Workspace namespace binding;
- file-browser continuation is scroll-driven rather than a visible manual **Load more** workflow;
- revision-stale cursors refresh the current directory instead of surfacing a generic continuation error;
- genuine paging failures stop automatic retries and retain explicit retry state;
- modal return behavior is scoped to the flow that opened the modal;
- Agent Access/client-adapter surfaces remain usable at the target desktop viewport.

## Agent Task safety model remains unchanged

rc.10 does not broaden Agent Task preparation or publication semantics:

- preparation copies only explicitly selected source files;
- preparation does not mutate source bytes or expand source MCP exposure;
- publication starts with nothing selected;
- only `fail` and `rename` conflict handling are exposed;
- overwrite remains unavailable;
- publication uses create-only writes;
- stale approved previews are rejected;
- committed destination bytes are verified;
- successful publication creates a unique publication receipt;
- partial multi-file failure reports cleanup-required state rather than claiming unsafe rollback.

Agent Tasks v1 remain limited to a read-write Local Filesystem Agent Workspace even though Agent Workspaces themselves can cover a broader set of writable OpenDAL backends.

## Product validation

Only after the canonical rc.10 Release and automatic downstream `Post Release Validation` both pass should `v0.8.1-rc.10` become the candidate for the resumed real pilot.

The pilot starts from representative v0.8.0 local state and installs rc.10 over it. It must cover:

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

See `docs/agent-tasks-pilot.md` for the complete protocol. No completed real rc.10 pilot is claimed by this release preparation.

## Release validation

Before creating the `v0.8.1-rc.10` tag, the exact final release-preparation PR head must pass all six mandatory workflows:

- CI;
- Integration Tests, including the release-critical activation, secret-migration, and backup/restore state smokes;
- Release Rehearsal;
- Coverage;
- Dependency Audit;
- Repo Lint, including the zero-manual release-policy assertions.

Only after those exact-head checks pass may the release-preparation PR merge and the annotated tag be created on the resulting main merge commit.

After tagging, the canonical Release workflow must pass the complete chain, including:

- signing-policy checks;
- frontend/UI/Rust/desktop state/storage/consistency/policy gates;
- Linux, macOS, and Windows packaging;
- updater-signature verification;
- curated SBOM generation and bundled-sidecar augmentation;
- checksums, provenance, and installer smoke checks;
- draft-release asset validation;
- publication and published-release asset re-download validation.

The automatic downstream `Post Release Validation` workflow must also pass before rc.10 is accepted for the resumed real pilot.

## Known boundaries

- v0.8.0 remains the stable public release while rc.10 is evaluated.
- rc.8 and rc.9 remain immutable failed candidate tags and are not moved or reused.
- rc.9 passing its gates and platform builds is release evidence, not completed product-pilot evidence.
- Agent Tasks v1 require a read-write Local Filesystem Agent Workspace.
- Storage browsing alone does not expose storage to agents.
- Agent Workspace creation alone does not imply that normal Agent Access is active.
- The built-in MCP HTTP transport remains loopback-oriented in the guided path; explicitly retained broader HTTP configurations require Advanced MCP review and existing authentication/start protections.
- Advanced MCP configurations with broader tools remain supported, but the guided Agent Access path intentionally refuses to activate a new workspace into them without explicit review.
- Local filesystem symlink/reparse-point defenses retain their documented check-then-OpenDAL TOCTOU limitation.
- Agent Task publication intentionally has no overwrite mode.
