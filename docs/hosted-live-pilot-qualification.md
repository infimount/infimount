# Infimount v0.8.2 C2.9k: Linux synthetic-only live pilot integration

Status: **operator-only implementation, disabled in normal builds**. No
live provider request, billable run, upload of real user data, or successful
provider interoperability is claimed by this implementation or its CI.

## Security and activation boundary

The desktop `hosted-live-pilot` Cargo feature is disabled by default. Stable
v0.8.1 remains unchanged, and the normal desktop never offers a paid action.

The opt-in build accepts **only** a prepared Agent Task whose single input is
`inputs/synthetic.txt` with the exact UTF-8 contents
`Infimount synthetic hosted pilot v1\n`. It rejects any different hash,
byte length or path. The separately reviewed brief must exactly match:

- Title: `Infimount synthetic pilot`
- Objective: `Read the public synthetic input and write the approved report.`
- One requested output: `outputs/report.md`

These restrictions are checked before accessing the provider credential and
before any paid session. The control plane independently verifies actual
prepared bytes and compares them with the declared SHA-256 under pinned
no-follow handles before and after native approval.

## End-to-end authority chain

1. Desktop resolves the task's configured local storage and workspace under
   lifecycle, registry and workspace mutation locks.
2. A native no-follow reader verifies the prepared manifest and the strict
   synthetic-only fixture gate. A key is read inside the trusted process from
   the OS keyring into a zeroized buffer.
3. The control-plane coordinator builds a bounded request from actual
   prepared file bytes. Native GTK review displays the complete redacted
   instructions, every input file, size and SHA-256, output names, fixed model
   `gpt-6-astra`, a fixed small hosted container, U.S.-only/non-ZDR disclosure and the US$1 best-effort
   spending request. Cancel and window closure fail closed.
4. Only an explicit response from the native in-process dialog creates an
   uncloneable, one-use consent capability. The coordinator pins and rehashes
   the exact input and output scope again. Any change aborts without a
   provider call.
5. A fixed-origin HTTPS transport submits at most one Agents API session,
   with environment network egress disabled, no mounts or MCP, streaming
   bounds, no redirects or automatic retries. A possibly charged unknown
   create remains unresolved rather than being retried.
6. A prepared-but-never-submitted attempt can be discarded locally after
   rechecking an empty attached output directory, without any provider call.
   A Running attempt without a captured provider ID remains blocked for
   manual reconciliation, never automatic retry.
7. The coordinator verifies the root turn, saved tool exit codes, artifact
   identities, exact output paths and actual downloaded bytes. It confirms
   the public session deletion and persists a separate create-only deletion
   receipt before any output import.
8. A descriptor-confined create-only importer writes to task outputs. All
   outputs require separate review. Publication back to original storage
   remains a different native human approval and cannot be authorized by
   the hosted agent.

## Interrupted-session deletion contract

An explicit **second native confirmation** is required for an interrupted
attempt with a privately persisted session ID. The native cleanup-only
operation never creates a new session or resends input bytes. It first
retrieves the exact authenticated provider session.

- `idle` or `failed`: request deletion of that session and require a matching
  `agent.session.deleted` confirmation.
- `in_progress`, `requires_action` or `budget_reached`: submit one
  `agent.session.input.cancel` event to the same fixed-origin session events
  endpoint. The HTTP 202 is **acceptance only**. Poll bounded session state
  until the same session is idle or failed before attempting DELETE.
- Unknown status, mismatched ID, cancel failure, pending cancellation,
  deletion failure or missing private receipt: leave the attempt Running
  and require recovery. Never retry a possibly paid create.
- A successful API deletion receipt proves only removal from the public API.
  It never proves immediate physical erasure, retention expiry, or success
  of the abandoned agent turn.

Official API references:
- <https://developers.openai.com/api/docs/guides/agents-api/sessions>
- <https://developers.openai.com/api/reference/resources/beta/subresources/agents/subresources/sessions/methods/delete>
- <https://developers.openai.com/api/reference/resources/beta/subresources/agents/subresources/sessions/subresources/events/methods/create>

## Reproducible operator preparation (offline)

Use an isolated Linux test account with its own native keyring and local
storage. Do not run the pilot build against a user's existing configuration
or publish it as a normal update. The ordinary build still rejects the
hosted execution command.

Start with the exact fully qualified source tree. The #255 correction
restored the private module boundary and made main match the eight-workflow
qualified #246 source tree on 27 November 2026. Verify any subsequent
candidate independently before live use.

From the repository root, with the project's Node.js 24, pnpm 10, Rust
1.94.0 and Linux Tauri prerequisites installed:

```bash
pnpm install --frozen-lockfile
node scripts/prepare-mcp-sidecar.mjs
cargo +1.94.0 check -p infimount --all-targets --features hosted-live-pilot --locked
cargo +1.94.0 test -p infimount --features hosted-live-pilot --locked
node --test scripts/qualify-openai-hosted-synthetic.test.mjs
```

These checks do not use a provider credential or make a paid request.
Create the sole public fixture in a fresh directory:

```bash
fixture_dir="$(mktemp -d)"
printf '%s\n' 'Infimount synthetic hosted pilot v1' > "$fixture_dir/synthetic.txt"
wc -c "$fixture_dir/synthetic.txt"
sha256sum "$fixture_dir/synthetic.txt"
printf 'Local source directory: %s\n' "$fixture_dir"
```

The file must contain exactly **36 bytes**, including the terminating LF,
with SHA-256
`96d1e197e24dfadde8ede461f4933f3a53dbc37618f3616951a6f86bf94fa5ef`.
Do not add other inputs, edit the task manifest or use real source data.

Launch the opt-in development build following the normal Tauri setup in
[Building from Source](../README.md#-building-from-source). Run the
frontend dev server separately if it is not already running:

```bash
pnpm --dir apps/desktop dev
# In a second terminal, from the same checkout:
pnpm --dir apps/desktop tauri dev --features hosted-live-pilot
```

1. Register the fresh fixture directory as Local Filesystem storage. Create
   a separate read-write Agent Workspace on local storage for the prepared
   task. Do not broaden source MCP access.
2. Select only `synthetic.txt` and use **Prepare Agent Task**. Use the exact
   title, objective and requested output listed above. Commit through
   Infimount's normal preparation flow. Do not handcraft a manifest, launch
   another agent, or modify the copied bytes.
3. Reopen the committed task in Agent Workspace Tasks. Open **Review remote
   sandbox export**, enter the same exact reviewed title, objective and
   output, then select **Preview export scope (no upload)** and **Verify
   actual prepared bytes (no upload)**. Check the input path, 36-byte size,
   SHA-256, sole requested output and provider disclosures. Both actions
   are local-only. In a normal build there must be no run action.

Stop here unless the live pilot has been separately authorized. A saved
credential, a successful build, or a verified preview is not permission
to transfer data or spend money.

## Authorized live qualification and evidence

Only an operator who has independently accepted the current provider terms,
U.S.-only residency, non-ZDR retention and potentially more than US$1 in
actual charges may continue. Use a dedicated project with independently
configured billing controls and a restricted key. Keep the key out of the
shell history, repository, task files and evidence.

- Save the key through **Optional hosted compute credentials** in Agent
  Workspace Tasks. The native keyring is the only credential source.
- After fresh byte verification, select **Review natively and run synthetic
  pilot**. Inspect the native dialog's complete scope, model, small
  container, provider terms and best-effort spending request. Only explicit
  approval there may submit one session. Cancel is not execution.
- On completion, refresh private execution history and review the task
  outputs. Require a succeeded attempt, matching API-level session deletion
  receipt, exactly one imported `outputs/report.md`, and a successful
  independent output-byte review. Verify the original synthetic source is
  unchanged and that no publication occurred.
- Record the exact build commit, input and output SHA-256 and byte counts,
  local attempt identifier, terminal status, API deletion result, whether
  the report meets the task, provider project activity and observed billing
  if available. Keep provider session IDs, credentials and raw provider
  responses private. Do not invent missing cost or retention evidence.
- If interrupted, follow the recovery contract above. Never repeat a
  possibly charged create automatically. Unknown-session-ID attempts
  require manual project reconciliation. Preserve partial local outputs.

A successful synthetic run is only an interoperability observation. It
does not qualify arbitrary user data, repeated production workloads, other
operating systems, or original-storage publication.

## Remaining qualification gates

- The combined opt-in desktop implementation and restored merge tree
  passed eight offline workflows. Subsequent changes must requalify.
- The actual separately authorized synthetic-only live API pilot remains
  pending. No paid request or real-data upload is claimed.
- macOS and Windows require independent safety qualification before hosted
  execution is enabled there. Original-storage publication remains separate.

No remote inference, paid session or upload is run in CI. The purpose of
this document is qualification of the dormant opt-in implementation, not
release authorization.
