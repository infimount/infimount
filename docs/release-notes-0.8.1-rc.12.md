# Infimount 0.8.1-rc.12: preserve the Linux release upgrade boundary

Release: https://github.com/infimount/infimount/releases/tag/v0.8.1-rc.12

Infimount 0.8.1-rc.12 carries the same product behavior as rc.11 and fixes the canonical Linux release-job ordering exposed by the first rc.11 tag.

## Release outcome

rc.12 is published and fully validated through the automated release boundary.

- the canonical Release workflow passed;
- Linux, macOS, and Windows builds passed;
- Linux exercised the exact freshly built rc.12 package through a real v0.8.0-to-rc.12 installer-over-install flow before the candidate-installing artifact smoke;
- updater signatures, SBOM coverage, checksums, install scripts, provenance, draft re-download, publication, and published-release re-download validation passed;
- automatic Post Release Validation passed;
- the post-release job re-downloaded the public rc.12 Linux package and independently repeated the v0.8.0-to-rc.12 installer exercise with user configuration untouched and storage state retained.

The remaining product-validation work is deliberately separate: one real coding task, one document task, and one data-analysis task must be judged materially useful by a human. That evidence informs stable promotion but is not a manual release-test gate.

## Why rc.12

`v0.8.1-rc.11` was created from the intended qualified `main` commit. Before tagging, its release-preparation head passed CI, Integration Tests, Release Rehearsal, Coverage, Dependency Audit, and Repo Lint, and the release branch passed the real desktop Release Pilot.

The canonical rc.11 Release then established that:

- every deterministic Release Gate passed;
- the macOS platform build passed;
- the Windows platform build passed;
- the Linux candidate package was built successfully;
- the ordinary Linux release-artifact smoke passed, including packaged-sidecar validation.

The Linux job then stopped at the newly added previous-stable-to-candidate installer-over-install gate.

The failure was deterministic and occurred before the upgrade path itself was exercised. `scripts/smoke-linux-release-artifacts.sh` intentionally installs the candidate `.deb` so it can inspect the installed executable. The upgrade harness was ordered after that step and correctly refused to mutate an already-installed Infimount package that it had not created:

```
Linux installer upgrade pilot failed: runner already has infimount installed; refusing to mutate an unknown installation
```

Because Linux failed before artifact upload, the publish job was skipped and no rc.11 GitHub Release was created.

rc.11 therefore remains an immutable failed candidate. It is release-orchestration evidence, not completed real-pilot evidence.

## Fix

PR #123 corrects the workflow boundary rather than weakening the installer harness.

The Linux release job now runs in this order:

1. build and collect the candidate Linux artifacts;
2. run the previous-stable-to-candidate installer-over-install pilot on the clean runner;
3. let that harness remove every package it installed;
4. run the normal Linux artifact smoke, which may install the candidate for executable inspection;
5. upload Linux artifacts only after both checks pass.

The harness still fails closed if Infimount is already installed before it starts. That guard remains important because silently removing or replacing an unknown runner installation would invalidate the evidence.

## Ordering is now policy, not convention

The zero-manual release-policy checker now asserts that `smoke-linux-upgrade-install.sh` appears before `smoke-linux-release-artifacts.sh` in the canonical Release workflow.

This converts the ordering dependency into a repository-enforced invariant rather than relying on a comment or reviewer memory.

PR #123 itself passed:

- CI;
- Integration Tests;
- Release Rehearsal;
- Coverage;
- Dependency Audit;
- Repo Lint;
- the dedicated Installer Upgrade Harness, exercising the already-published v0.8.0-to-rc.10 package path.

## Product behavior remains rc.11-compatible

rc.12 does not introduce another storage, Agent Access, Agent Task, or MCP authorization model change.

It retains the already-automated release-pilot coverage for:

- legitimate storage-only onboarding;
- legacy `~` Local Filesystem root normalization at first workspace binding;
- first-class Agent Access;
- least-privilege read-only and read-write tool exposure;
- stdio versus HTTP runtime semantics;
- persisted fail-closed general Agent Access;
- independently scoped `serve-agent-task`;
- automatic file-browser continuation and stale-cursor recovery;
- Agent Task preparation, review, stale-preview rejection, fail/rename conflict handling, destination verification, and unique publication receipts;
- packaged-sidecar confinement;
- v0.8.0-to-candidate Linux installer-over-install retention checks.

The three real coding, document, and data-analysis workload usefulness judgments remain human product-validation evidence. Deterministic CI is not a substitute for those judgments.

## rc.12 release validation completed

The exact rc.12 release-preparation head passed:

- CI;
- Integration Tests;
- Release Rehearsal;
- Coverage;
- Dependency Audit;
- Repo Lint.

The release branch also passed the real desktop Release Pilot.

After tagging, the canonical Release passed:

- every deterministic Release Gate;
- the exact freshly built Linux candidate through the previous-stable-to-candidate installer-over-install pilot before any candidate-installing artifact smoke;
- Linux artifact smoke;
- macOS and Windows platform builds;
- artifact collection and checksum/SBOM/signature validation;
- draft re-download validation;
- publication;
- published-release re-download validation.

The automatic downstream `Post Release Validation` must then pass, including the published-package Linux installer-over-install check.

Those checks are green. rc.12 is the active final real-pilot candidate.

## Known boundaries

- `v0.8.0` remains the stable public release until stable promotion is explicitly completed.
- rc.8, rc.9, and rc.11 remain immutable failed candidates and are never moved or reused.
- rc.10 remains a successfully published candidate and important validation evidence.
- Automated release-pilot success is deterministic product/safety evidence, not a substitute for human assessment of model-output usefulness.
- Agent Tasks v1 still require a read-write Local Filesystem Agent Workspace.
- Agent Task publication intentionally has no overwrite mode.
- Storage browsing alone does not expose storage to agents.
- Guided Agent Access remains local and least-privilege; broader tools or non-loopback HTTP require Advanced MCP review.
