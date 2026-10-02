# Infimount 0.8.1-rc.13: final release-infrastructure qualification

Release: https://github.com/infimount/infimount/releases/tag/v0.8.1-rc.13

Infimount 0.8.1-rc.13 carries the same application/runtime behavior as rc.12. It exists to qualify the exact release, publication, documentation, and stable-promotion machinery that will be used for v0.8.1.

## Why rc.13

rc.12 already proved the intended product behavior and package-upgrade boundary:

- every deterministic Release Gate passed;
- Linux, macOS, and Windows package builds passed;
- v0.8.0-to-rc.12 Linux installer-over-install validation passed before publication;
- draft upload/re-download validation passed;
- updater signatures, checksums, SBOM, provenance, install scripts, and package-sidecar integrity passed;
- publication and public-asset re-download validation passed;
- automatic Post Release Validation independently repeated the published-package upgrade check.

After rc.12, no application/runtime source changed. The repository did, however, receive release-facing maintenance that stable should not exercise for the first time:

- `actions/cache@v5`;
- `actions/setup-go@v6`;
- `actions/download-artifact@v8`;
- `actions/attest-build-provenance@v4`;
- `actions/deploy-pages@v5`;
- stronger published-prerelease documentation validation;
- stable-document promotion based on an explicit candidate marker instead of mutable prose;
- corrected public security, release, governance, migration, Agent Task, and landing-page documentation.

rc.13 is therefore an infrastructure/promotion candidate, not another product-feature candidate.

## Product behavior

The application behavior remains the rc.12 behavior:

- legitimate storage-only onboarding;
- first-class Agent Access with explicit stdio/HTTP semantics;
- fail-closed general Agent Access;
- scoped Agent Workspaces and independently scoped `serve-agent-task`;
- legacy `~` Local Filesystem normalization at first workspace binding;
- automatic pagination and stale-cursor recovery;
- bounded Agent Task preparation and review;
- stale-preview rejection;
- fail/rename publication conflict handling;
- no overwrite mode;
- create-only publication, destination verification, and unique receipts;
- packaged sidecar confinement;
- v0.8.0-to-candidate installer-over-install retention checks.

## rc.13 qualification

Before the tag is created, the exact release-preparation head must pass:

- CI;
- Integration Tests;
- Coverage;
- Dependency Audit;
- Repo Lint;
- Release Rehearsal;
- Installer Upgrade Harness;
- the real desktop Release Pilot.

After tagging, the canonical Release must prove the current release workflow on a real immutable candidate, including:

- all deterministic Release Gates;
- the previous-stable-to-candidate Linux installer upgrade before candidate-installing artifact smoke;
- Linux, macOS, and Windows package builds;
- updater-signing key correspondence;
- `actions/download-artifact@v8` artifact collection;
- `actions/attest-build-provenance@v4` provenance emission;
- checksums, SBOM, updater metadata/signatures, install scripts, and package validation;
- draft re-download validation;
- publication;
- published-release re-download validation.

Automatic Post Release Validation must then pass against the published rc.13 assets, including the public-package Linux upgrade check and the published release-note URL guard.

## Final product-validation boundary

Automated success does not establish model-output usefulness. After rc.13 is published and all automated release checks are green, the final promotion evidence is:

1. one real coding Agent Task;
2. one real document-synthesis Agent Task;
3. one real data-analysis Agent Task.

The deterministic workload validators may verify structure, calculations, patch applicability, source non-mutation, publication safety, and evidence shape. A human must still decide whether each result is materially useful and correct before real pilot evidence can be marked passed.

## Signing boundary

Every release requires cryptographically signed updater artifacts. macOS platform signing/notarization and Windows Authenticode are exercised when their complete signing credentials are configured; otherwise the release explicitly records that the platform package is unsigned. The rc.13 run is the correct place to qualify any newly configured platform-signing credentials before stable promotion.

## Known boundaries

- v0.8.0 remains the stable public release until v0.8.1 stable promotion completes.
- rc.8, rc.9, and rc.11 remain immutable failed candidates and are never moved or reused.
- rc.10 and rc.12 remain successfully published validation candidates.
- Agent Tasks v1 require a read-write Local Filesystem Agent Workspace for writable task outputs.
- Agent Task publication intentionally has no overwrite mode.
- Storage browsing alone does not expose storage to agents.
- The documented local-filesystem TOCTOU boundary remains part of the current local-process trust model.
