# v0.8.2 C1.3a: memory-only verified sandbox ingest

Status: **isolated, provider-independent, in-memory proof of the byte-copy
contract**. No external sandbox adapter, filesystem write, MCP write tool or
trusted production import service is introduced.

## Transaction model

`infimount_control::sandbox_memory_ingest::ingest_sandbox_output_into_memory`
accepts a sandbox output **candidate** from C1.2, one isolated OpenDAL
Memory staging operator, a separate Memory destination operator and a
trusted task ID.

The operation is deliberately restricted to `Scheme::Memory` for both
operators. An arbitrary filesystem or cloud operator fails closed.
A same-user process does not gain a real storage write capability by
calling this function.

The function:

1. Recomputes and compares the entire C1.2 candidate, including its
   declared metadata fingerprint. Neither fingerprint nor candidate
   grants authority.
2. Recursively inventories exactly the staged `outputs/` file tree,
   rejects surprise files, missing files and declared-size mismatches,
   and retains the existing file-count and byte limits.
3. Independently hashes the actual in-memory file contents before any
   destination write. A same-size file substitution fails.
4. Derives every destination from the trusted task UUID and validated
   `outputs/`-relative path. Rejects an existing destination before
   starting the copy.
5. Reuses `publish_reviewed_agent_task_file`, which rehashes source
   bytes before and during streaming, requires create-only writes, and
   independently rehashes committed destination bytes afterward.
6. Never deletes ambiguous files on failure. If an error follows an
   earlier committed file, return `TransferCleanupRequired`.

The result is **in-memory ingest evidence only**, not a desktop
publication receipt, provider identity attestation or human approval.

## Deliberately unresolved prerequisites

This is a bounded real-byte regression harness, not a production importer.
It does not claim that remote providers report truthful paths or types,
that arbitrary host filesystem traversal is symlink/reparse-point safe,
or that an unrestricted same-UID agent cannot bypass Infimount.

A later reviewed C1.3b production importer MUST:

- Qualify no-follow path opening and containment on each supported OS,
  including all parent components, symlinks, reparse points and races.
- Acquire the trusted workspace mutation lock across enumeration,
  exact-byte copy, persistent execution-evidence write and cleanup.
- Use the existing secure task workspace identity and operator handling.
  Do not accept an agent/provider-selected destination.
- Enforce create-only verified copy, including late-collision and
  partial-commit/receipt failure behavior.
- Persist private execution evidence and require separate human review
  before final publication into systems of record.
- Pass the full seven-workflow exact-head gate, plus actual provider
  ingress/egress adversarial qualification.

These are **not** delegated to a generic file adapter, source workspace
or sandbox. No headless publication, model-callable approval issuer or
new runtime authority is introduced.

Related: [C1.1 outbound bundle](sandbox-task-bundle.md),
[C1.2 candidate validator](sandbox-output-import.md), and
[governed agent data-plane roadmap](roadmaps/v0.8.2-governed-agent-data-plane.md).
