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

## Remaining qualification gates

- CI for the component PRs and the combined feature-gated pilot must pass,
  including the new `Hosted Pilot Qualification` workflow.
- Independent review of the complete native approval and transport flow.
- One **separately authorized**, synthetic-only API interoperability pilot
  with explicit acceptance of provider billing, U.S. data residency and
  non-ZDR retention. The US$1 limit is a best-effort provider control, not
  a guarantee. This is **not** permission to run the pilot.
- A pinned-session recovery path for crashes occurring before deletion and
  independently qualified macOS/Windows safety semantics before enabling
  those operating systems.

No remote inference, paid session or upload is run in CI. The purpose of
this document is qualification of the dormant opt-in implementation, not
release authorization.
