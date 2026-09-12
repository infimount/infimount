# Agent Tasks v0.8.1 pilot protocol

This protocol validates whether Agent Tasks are useful and understandable in real work after the automated release gates have already established build, test, packaging, and safety-contract correctness.

Pilot evidence is a **product-validation artifact**, not a manual release-test gate. A synthetic fixture, CI result, mocked UI test, or release rehearsal must never be presented as a completed pilot.

## Objective

The v1 pilot answers five questions:

1. Can a user prepare a bounded snapshot without changing the source or expanding source MCP access?
2. Can Codex complete useful work through the existing Infimount workspace MCP boundary?
3. Can the user understand reviewed outputs well enough to make an explicit publication decision?
4. Do create-only publication, fail/rename conflict handling, and stale-preview rejection behave safely in a real desktop flow?
5. Can an existing v0.8.0 installation move to the v0.8.1 candidate without losing local configuration, storage registrations, or Agent Workspaces?

The pilot does not justify new feature breadth. A failed run should first identify whether the cause is an Infimount defect, a client/agent limitation, a task-quality problem, or an environment problem.

## Candidate prerequisite

Once published, use the `v0.8.1-rc.5` candidate for the resumed pilot.

`v0.8.1-rc.1` was published successfully, but its first real pilot attempt stopped during Agent Workspace setup before any Agent Task was executed. That run exposed a material Infimount workflow defect: workspace creation still carried agent-type/template and second-path concepts, and a shell-style Local Filesystem root could fail late during namespace binding. rc.2 fixed that product blocker. rc.1 must not be presented as completed pilot evidence.

`v0.8.1-rc.2` was then tagged at the intended candidate commit, but its Release workflow failed twice in the Linux packaging job before Infimount compilation because a GitHub-hosted runner contained an unrelated Google Chrome APT repository with inconsistent package metadata (`Hash Sum mismatch`). The dependent publish job remained skipped, so no rc.2 GitHub Release was published. The product/release gates and macOS/Windows builds passed; the failure was classified as release-environment infrastructure. rc.3 carried the same product behavior plus release APT-source hardening. rc.2 must not be presented as completed pilot evidence.

`v0.8.1-rc.3` was published successfully and its canonical Release workflow passed all release gates, multi-platform packaging, publication, and public-asset re-download validation. The separate `Post Release Validation` workflow then failed before artifact validation because its `workflow_run` context supplied reserved `GITHUB_REF_NAME=main`; the attempted step-level override did not replace that reserved variable, so the version-sync script rejected `main` instead of using the resolved rc.3 tag. PR #99 fixed that orchestration path by passing the resolved release tag explicitly and adding a regression test. rc.4 carried the same product behavior plus that post-release validation fix. rc.3 must not be presented as completed pilot evidence.

`v0.8.1-rc.4` was published successfully. Its canonical Release workflow and automatic downstream `Post Release Validation` both passed. The resumed real pilot then exposed product workflow and guided-access defects before a complete three-workload pilot could be recorded: legacy Local Filesystem home-alias roots could still fail Agent Workspace binding, onboarding forced normal users through advanced MCP administration, modal state could relaunch onboarding unexpectedly, the client-adapter step could exceed the usable viewport, and the guided agent-access path needed stronger least-privilege handling for per-workspace readiness, extra global tools, and non-loopback HTTP listeners. PR #102 fixes those rc.4 pilot blockers. rc.4 must not be presented as completed pilot evidence.

Record:

- prerelease version;
- exact Git commit SHA;
- operating system used for the run;
- previous installed stable version;
- agent client name.

The resumed pilot should start from an installed v0.8.0 environment and install v0.8.1-rc.5 over it. If the prerelease is not offered through the stable updater channel, installer-over-install is the correct candidate upgrade exercise. Do not claim that a prerelease installer proves stable-channel updater behavior.

For the controlled task workspace in this pilot, continue to use an **explicit absolute host path**. This keeps the Agent Task workload evidence independent from the separate legacy-root regression below. Shell-variable notation such as `$HOME/...` remains invalid. rc.5 also supports legacy `~` and `~/...` Local Filesystem roots by canonically normalizing them once before the first workspace namespace binding.

## rc.5 focused regression checks

Before the three workload pilots, verify the product corrections that caused rc.5 to exist.

### Legacy Local Filesystem home alias

Use a v0.8.0 Local Filesystem storage whose configured root is the legacy `~` form and that has no bound Agent Workspace. After upgrading to rc.5:

1. confirm ordinary storage browsing still works;
2. create one Agent Workspace on that storage through the normal UI;
3. verify workspace creation succeeds without manually editing the storage first;
4. verify the storage root is normalized to the concrete canonical home directory before namespace binding;
5. verify existing storage contents remain available and no unrelated storage settings are changed.

Do this after the upgrade-retention snapshot is captured, because the one-time normalization is an intentional post-upgrade mutation.

### Guided Agent Access

Exercise the normal onboarding flow as **Storage → Workspace → Agent Access → Client → Verify**.

Verify all of the following:

- the selected workspace must be explicitly prepared; another exposed workspace on the same storage does not make it ready;
- a first-time read-only setup enables only the read tools;
- a first-time read-write setup adds only `mkdir` and `write_file` beyond the read tools;
- when MCP is disabled, guided setup chooses local stdio rather than activating a drafted HTTP listener;
- an already-active MCP server with additional global tools is rejected by the guided path and directs the user to Advanced MCP settings;
- an already-active HTTP server bound beyond loopback is rejected by the guided path and directs the user to Advanced MCP settings;
- broad default storage access or additional manual storage grants are rejected by the guided path;
- closing Add Storage, Agent Workspaces, or Advanced MCP only returns to onboarding when onboarding explicitly opened that dialog;
- the MCP client-adapter step remains usable at the target desktop viewport without inaccessible controls below the fold.

These checks validate the rc.5 corrections. They do not replace the three real Agent Task workloads below.

## Evidence privacy boundary

Pilot evidence must not contain:

- original source paths or host paths;
- storage IDs or full storage configuration;
- credentials, OAuth values, tokens, or secrets;
- source file contents;
- copied output contents.

Record only opaque task IDs, storage backend kinds, counts, byte totals, hashes, relative evidence references, and concise observations.

The validator rejects known sensitive/source-path field names. Human observations still require normal judgment. Do not paste secrets or private document content into `observations`.

## Evidence bundle

Create one JSON evidence bundle for one candidate/platform run. It must use `schemaVersion: 1` and `synthetic: false`.

Validate it with:

```bash
node scripts/check-agent-task-pilot-evidence.mjs /path/to/pilot-evidence.json
```

CI validates only the explicit synthetic fixture with:

```bash
node scripts/check-agent-task-pilot-evidence.mjs \
  tests/fixtures/agent-tasks-pilot/synthetic-evidence.json \
  --allow-synthetic
```

`--allow-synthetic` is for repository fixtures only. A real pilot bundle must pass without that flag.

## Source-selection digest

Each pilot records a before/after SHA-256 for the exact selected source bytes. The evidence file stores only the aggregate digest, never the source paths.

Use one deterministic method before and after the task:

1. enumerate the exact selected files;
2. calculate each file's SHA-256 and byte size;
3. sort records by the same normalized relative path ordering both times;
4. hash the same canonical record representation into one aggregate SHA-256;
5. store only the aggregate before/after values in the evidence bundle.

The two aggregate values must match. If the source backend or local tooling cannot produce a trustworthy repeatable digest, that run is not sufficient evidence for the source-nonmutation invariant.

## Pilot 1: coding

Use a small non-sensitive code slice with enough context to require real reasoning, not a single trivial file.

Recommended objective:

- diagnose one concrete bug or design issue;
- produce `outputs/review.md` with reasoning and affected behavior;
- produce `outputs/patch.diff` or replacement files that can be independently checked against a disposable copy.

Required observations:

- preflight item/file/byte counts are plausible;
- the prepared task contains only selected inputs;
- Codex is launched through Infimount's existing MCP integration;
- outputs appear under `outputs/`;
- no output is selected for publication by default;
- the proposed change is materially useful and can be checked against the disposable source copy;
- publication succeeds only after explicit review and approval.

## Pilot 2: document

Use several related non-sensitive documents, preferably from a different supported source backend when convenient.

Recommended objective:

- produce `outputs/summary.md`;
- produce `outputs/action-items.md` or another decision-oriented artifact;
- verify that important source facts are represented correctly and obvious unsupported claims are absent.

Use this run to exercise **rename** publication. Pre-create a destination with the same name, first verify that **fail** policy blocks the conflict, then review the rename plan and publish the renamed output.

## Pilot 3: data analysis

Use a bounded CSV or similar tabular dataset that has independently checkable totals.

Recommended objective:

- produce `outputs/findings.md`;
- produce `outputs/summary.csv`;
- compare at least the key totals or aggregates against an independent calculation.

The goal is not benchmark-level model evaluation. The goal is to prove that preparing data, handing it to the agent, reviewing generated artifacts, and explicitly publishing a useful subset forms a coherent workflow.

## Mandatory negative safety probes

These probes are part of the real pilot bundle because they test the user's actual safety boundary, not only the happy path.

### Existing destination under fail policy

1. Choose an output and a destination where the same target name already exists.
2. Use **fail** conflict policy.
3. Verify preview cannot authorize a destructive overwrite.
4. Record `safetyProbes.failConflictRejected: true` only after observing the rejection.

### Stale approved preview

1. Review an output and obtain a publication preview.
2. Change the reviewed output bytes before applying that preview.
3. Attempt publication using the stale approval.
4. Verify publication is rejected.
5. Re-review and re-preview before any eventual successful publication.
6. Record `safetyProbes.stalePreviewRejected: true` only after the stale plan is rejected.

### No overwrite mode

Verify the real publication UI does not expose an overwrite mode. Record `safetyProbes.overwriteUnavailable: true` only after observing the candidate UI.

## Upgrade exercise

Start from v0.8.0 with representative local state, then install v0.8.1-rc.5 over it.

The evidence bundle requires all of the following:

- general application configuration remains usable;
- storage registry entries remain present;
- Agent Workspace registry entries remain present;
- Agent Tasks are visible and usable after the upgrade;
- the upgraded application starts normally.

Capture the post-upgrade retention snapshot before exercising the rc.5 legacy `~` normalization check. The upgrade check is a candidate-compatibility exercise. It does not replace automated migration tests and does not claim stable-channel updater behavior unless that exact updater path was actually used.

## Per-task evidence

Each of the three task records contains:

- workload class;
- task UUID;
- source backend kind and local workspace kind;
- source MCP exposure before and after;
- aggregate source-selection SHA-256 before and after;
- preflight selected item, expanded file, and byte counts;
- reviewed output file and byte counts;
- confirmation that publication started with nothing selected;
- publication selection count and fail/rename policy;
- confirmation that overwrite was unavailable;
- confirmation that the exact preview was approved;
- confirmation that committed destination bytes were verified;
- unique publication receipt path and SHA-256;
- at least two relative UI evidence references, covering reviewed publication state and successful publication;
- final task pass state.

`passed: true` means both the safety flow and the actual task objective were acceptable. A technically safe run with useless or clearly incorrect output is not a successful product pilot.

## Acceptance criteria

A candidate/platform evidence bundle passes only when:

- the rc.5 focused regression checks above pass without a material workflow or safety defect;
- coding, document, and data-analysis pilots all pass;
- all source before/after aggregate hashes match;
- source MCP exposure is unchanged for every task;
- every publication begins with nothing selected;
- published outputs were explicitly reviewed and approved;
- no overwrite mode is exposed;
- destination verification and a unique receipt are present;
- fail-on-conflict rejection is observed;
- stale-preview rejection is observed;
- v0.8.0 to candidate upgrade retention passes;
- the real evidence bundle passes `check-agent-task-pilot-evidence.mjs` without `--allow-synthetic`;
- no observation describes a material safety, data-loss, or workflow blocker.

A passing bundle is evidence for broader preview and stable-release planning. It does not automatically justify stable publication.

## Failure classification

Classify every failed pilot before changing scope:

- **Infimount safety defect**: source mutation, access expansion, overwrite path, stale approval accepted, incorrect destination verification, receipt inconsistency. Block promotion and fix before another candidate.
- **Infimount workflow defect**: confusing or broken prepare/review/publish flow that prevents a normal user from completing the task. Fix only the material blocker.
- **Client/agent limitation**: Codex behavior or model quality is the cause while Infimount boundaries work correctly. Record it separately from storage-product correctness.
- **Task-quality failure**: output is not useful or correct enough even though the workflow is sound. Improve task framing or reassess the product value hypothesis before adding features.
- **Environment failure**: external registry, provider, network, or OS issue occurs before the product path is exercised. Retry without changing product code unless the product should reasonably tolerate the condition.
