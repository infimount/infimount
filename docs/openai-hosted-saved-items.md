# Infimount v0.8.2 C2.3: offline hosted saved-item outcome audit

Status: **bounded offline structural validation only**. No HTTP, credentials,
provider calls, artifact downloads, storage mutation or publication.

## The gap after C2.2

A root turn completing is not proof that every tool succeeded. The
[Agents API saved-items endpoint](https://developers.openai.com/api/reference/resources/beta/subresources/agents/subresources/sessions/subresources/turns/subresources/items/methods/list)
returns paginated `data`, `first_id`, `last_id`, and `has_more`.
A trusted caller must fetch all pages in ascending order, passing the previous
`last_id` as the next `after` cursor, for a pinned session/root turn.

`openai_hosted_items::HostedSavedItemAudit` provides an offline gate:

- Check expected root turn ID on each item. The trusted transport separately
  binds the authenticated session and actual root-turn response.
- Reject duplicate item IDs, incorrect turn, invalid page boundaries, cursor
  drift, malformed JSON, partial pagination, and unexpected extra pages.
- Bound each raw page to 2 MiB and 100 items, at most 50 pages / 5,000
  items total. Do not retain item content.
- For the restricted C2.1 pilot, permit completed messages and reasoning
  items, plus completed command executions with an explicit zero exit code.
  Reject unknown tool types, unfinished/failed results, and nonzero or
  absent command exit codes. This intentionally rejects recoverable failed
  commands pending a more precise provider-specific verification contract.
- Return count-only audit evidence **only after** complete pagination and
  C2.2 `RootTurnCompletedUnverified`. The returned summary is **not**
  permission to download, import, or publish artifacts.

## Still required before remote execution

1. An authenticated, pinned session/turn transport that fetches complete
   event and saved-item state with bounded retries and cancellation/cleanup.
2. Separately reviewed provider export consent explaining U.S. data
   residency/retention, exact files and bytes, provider and expected cost.
3. Trustworthy artifact enumeration and download restricted to
   `/workspace/outputs/`, with C1.2/C1.3 actual-byte rehashing, no-follow
   handling, task locking and create-only import.
4. Private execution evidence, expiration and user-controlled deletion.
5. Separate human review before any original storage publication.

Neither a provider event nor an audit summary is a user approval grant.
The local-first browser remains functional without an account or upload.
Keep stable v0.8.1 unchanged and native headless publication disabled.
