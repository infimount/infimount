# Infimount 0.8.1-rc.11: automate the release pilot

Release: not published yet.

Infimount 0.8.1-rc.11 carries the same product behavior validated in rc.10 and turns the remaining manual release-confidence checks into deterministic automation.

## Why rc.11

`v0.8.1-rc.10` was published successfully. Its canonical Release workflow passed every release gate, Linux/macOS/Windows packaging, draft re-download validation, publication, and published-release re-download validation. The automatic downstream `Post Release Validation` also passed.

The resumed real validation then confirmed important product behaviors on the actual rc.10 install:

- rc.7 to rc.10 installer-over-install retained the existing storage, workspace, MCP, and app settings byte-for-byte before first launch;
- storage-only onboarding legitimately completed from a previously skipped state without changing storage/workspace/MCP exposure;
- onboarding stayed closed on relaunch without a floating activation reminder;
- Agent Access remained available after storage-only onboarding;
- a legacy Local Filesystem `~` root normalized through real workspace creation without a manual storage edit;
- exactly one read-only workspace and its workspace-managed policy rule were added while unrelated storage/workspace/MCP state remained unchanged.

Those observations exposed no new product blocker. The remaining issue was release process cost: the same multi-step desktop and installer checks were still too manual and too easy to invalidate with shell-harness mistakes.

rc.11 therefore focuses on automation, not feature expansion.

## Real desktop release pilot

PR #117 added a dedicated `Release Pilot` workflow and `pnpm test:release-pilot`.

The pilot runs the actual Tauri desktop under WebDriver on Linux with:

- an isolated disposable HOME;
- a real transient Secret Service/keyring;
- the real Rust/Tauri command boundary;
- the packaged MCP sidecar;
- no developer configuration or credentials.

It automatically proves:

- previously skipped onboarding can reopen through Setup Guide;
- **Browse storage only** completes onboarding legitimately;
- storage-only completion does not change storage, workspace, or MCP settings;
- onboarding remains closed across relaunch;
- normal storage browsing remains usable;
- Agent Access remains available later;
- legacy `~` roots normalize only at first workspace binding;
- **Connect agent** preselects the chosen workspace;
- read-only Agent Access exposes only the six read tools;
- read-write Agent Access adds only `mkdir` and `write_file`;
- stdio remains client-launched/on-demand with no background Start action;
- HTTP uses explicit Start/Stop;
- ordinary `infimount_mcp serve` fails closed while general Agent Access is disabled;
- disabled guided setup returns to local stdio with the minimum read tool profile;
- the packaged-sidecar safety probe proves one allowed workspace read and one denied out-of-scope read.

No production Agent Access gate was weakened to make this test possible.

## Pagination and stale-cursor automation

PR #119 extended the real desktop pilot with the rc.5 pagination regression.

The harness creates a 260-file local folder, loads the first 200-entry page, then deliberately changes the storage revision through legitimate workspace/Agent Access preparation while the browser remains mounted.

The pilot then proves:

- the fallback **Load more** control remains screen-reader-only rather than becoming a normal visible workflow;
- scrolling automatically continues beyond page one;
- the old revision-bound cursor becomes stale by design;
- the browser internally restarts from page one rather than surfacing a generic continuation error;
- continuation resumes after that refresh;
- the last fixture entry is reached successfully.

This reproduces the exact class of stale-cursor condition seen during the earlier real pilot without adding a test-only backend bypass.

## Agent Task publication safety automation

The same PR extends the desktop pilot through a deterministic Agent Task prepare/review/publish flow.

The test verifies:

- one selected source file is copied into a read-write Local Agent Workspace;
- the source bytes remain unchanged;
- the source storage registry/MCP exposure remains unchanged throughout the task flow;
- the output review starts with nothing selected for publication;
- the publication UI does not expose overwrite;
- fail-on-conflict reports the existing destination and cannot publish destructively;
- **Keep both** produces a rename plan;
- changing reviewed output bytes after preview invalidates that approval;
- stale publication is rejected;
- refreshing outputs, re-reviewing, and re-previewing produces a fresh valid plan;
- the final renamed destination bytes match the reviewed output;
- exactly one unique publication receipt is created;
- the receipt contains task/destination metadata without leaking a host filesystem path.

The deterministic task content is test data only. It does not replace the human judgment required to decide whether real coding, document, and data-analysis outputs are actually useful.

## Independently scoped Agent Task MCP

PR #119 also proves the `serve-agent-task` boundary independently of general Agent Access.

While the persisted general Agent Access gate is disabled, the packaged sidecar is launched interactively using the exact prepared task/workspace scope. The pilot verifies that it:

- completes a real MCP initialize handshake;
- exposes exactly `list_dir`, `stat_path`, `read_file`, `search_paths`, `write_file`, and `mkdir`;
- reads the prepared task input inside the task scope;
- rejects a read outside the prepared task with `ERR_SESSION_FORBIDDEN`.

This confirms that the scoped Agent Task path remains independently authorized without reopening the general MCP surface.

## Packaged installer-over-install automation

PR #120 adds `scripts/smoke-linux-upgrade-install.sh` and an `Installer Upgrade Harness`.

The harness uses real Debian packages rather than source-tree state:

1. resolve the previous published stable release;
2. download the stable `Infimount-amd64.deb` and published checksums;
3. verify the stable package checksum;
4. install the stable package on a clean runner;
5. launch the installed stable desktop against isolated representative local state;
6. verify the packaged stable MCP sidecar reports the stable package version;
7. install the candidate `.deb` directly over stable;
8. verify installation itself did not mutate the isolated user configuration before candidate launch;
9. verify the installed candidate package version;
10. verify the candidate packaged MCP sidecar reports the candidate version;
11. launch the candidate;
12. verify the stable storage identity/exposure state remains retained.

The regression workflow already proved the published `v0.8.0 -> v0.8.1-rc.10` package path.

## Canonical Release now checks the exact Linux package

The Linux build in the tag-triggered canonical Release workflow now runs the same installer-over-install harness against the freshly built candidate `.deb` before platform artifacts are uploaded.

For a tag such as `v0.8.1-rc.11`, the check therefore exercises the exact candidate package produced by that release run, not a source-tree approximation.

If the package cannot upgrade the previous stable installation while retaining representative local storage state, the Linux platform build fails and publication remains blocked.

## Post Release Validation repeats the published package check

The automatic downstream `Post Release Validation` now downloads the published Linux candidate and repeats the previous-stable-to-candidate installer exercise.

This gives two independent package boundaries:

- pre-publication: the exact freshly built Linux candidate package;
- post-publication: the exact package re-downloaded from the GitHub Release.

The zero-manual release-policy gate requires both checks to remain wired into the workflows.

## Product behavior remains rc.10-compatible

rc.11 does not introduce another Agent Access or Agent Task model change.

It retains:

- legitimate storage-only onboarding;
- first-class Agent Access;
- Storage -> Workspace -> Agent Access -> Connect Client -> Safety Probe;
- local stdio as the guided first-time transport;
- explicit HTTP Start/Stop;
- least-privilege workspace readiness;
- fail-closed routing to Advanced MCP when authority is broader than the guided contract;
- persisted general Agent Access enforcement for ordinary `serve`;
- independently scoped `serve-agent-task`;
- canonical legacy-home root normalization at first workspace binding;
- automatic pagination continuation and stale-cursor recovery;
- create-only Agent Task publication with fail/rename conflict handling;
- stale-preview rejection and destination verification;
- unique publication receipts.

The only desktop-source adjustment introduced by the automation work is stable accessibility/test targeting on an existing transport control; it does not broaden capability or change security semantics.

## Human product validation that remains

CI can prove deterministic workflow and safety properties. It cannot decide whether generated work is genuinely useful.

After rc.11 is published and both Release and Post Release Validation pass, the remaining real product pilot is intentionally narrow:

- one coding Agent Task;
- one document Agent Task;
- one data-analysis Agent Task;
- Codex handoff through the scoped Infimount MCP path;
- human review that each output is materially useful/correct enough for its objective.

The real evidence bundle must use `synthetic: false` and pass:

```bash
node scripts/check-agent-task-pilot-evidence.mjs /path/to/pilot-evidence.json
```

without `--allow-synthetic`.

The automated deterministic Agent Task flow must not be represented as those three human workload passes.

## Release validation

Before creating `v0.8.1-rc.11`, the exact release-preparation head must pass:

- CI;
- Integration Tests;
- Release Rehearsal;
- Coverage;
- Dependency Audit;
- Repo Lint.

The merged release branch also runs the real desktop Release Pilot automatically.

After tagging, the canonical Release workflow must pass its complete existing chain plus the exact previous-stable-to-candidate Linux installer upgrade before Linux artifacts are uploaded.

The automatic downstream `Post Release Validation` must then pass, including the published-package installer upgrade.

Only after those checks are green should rc.11 be treated as the final real-pilot candidate.

## Known boundaries

- `v0.8.0` remains the stable public release until stable promotion is explicitly completed.
- rc.8 and rc.9 remain immutable failed candidates and are never moved or reused.
- rc.10 remains a successfully published candidate and important validation evidence.
- Automated release-pilot success is deterministic product/safety evidence, not a substitute for human assessment of model-output usefulness.
- Agent Tasks v1 still require a read-write Local Filesystem Agent Workspace.
- Agent Task publication intentionally has no overwrite mode.
- Storage browsing alone does not expose storage to agents.
- Guided Agent Access remains local/least-privilege; broader tools or non-loopback HTTP require Advanced MCP review.
