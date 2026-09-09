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

Use a published v0.8.1 prerelease built from a known commit, starting with `v0.8.1-rc.1`.

Record:

- prerelease version;
- exact Git commit SHA;
- operating system used for the run;
- previous installed stable version;
- agent client name.

The first pilot should start from an installed v0.8.0 environment and install the v0.8.1 candidate over it. If the prerelease is not offered through the stable updater channel, installer-over-install is the correct candidate upgrade exercise. Do not claim that a prerelease installer proves stable-channel updater behavior.

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

Start from v0.8.0 with representative local state, then install the v0.8.1 prerelease over it.

The evidence bundle requires all of the following:

- general application configuration remains usable;
- storage registry entries remain present;
- Agent Workspace registry entries remain present;
- Agent Tasks are visible and usable after the upgrade;
- the upgraded application starts normally.

The upgrade check is a candidate-compatibility exercise. It does not replace automated migration tests and does not claim stable-channel updater behavior unless that exact updater path was actually used.

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

This classification prevents a flaky external dependency or a weak agent answer from being misdiagnosed as a storage safety defect, and prevents a real safety failure from being dismissed as model variance.
