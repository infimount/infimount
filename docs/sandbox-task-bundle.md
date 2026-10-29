# v0.8.2 C1.1: sanitized sandbox task bundle contract

Status: **pure metadata projection only**, no remote upload, provider SDK,
authorization, runtime launch, or write-back implementation.

## Purpose

A remote sandbox must receive a copied, user-reviewed task, **not** the raw
local Agent Task manifest, workspace, storage identities or credentials.
`infimount_control::sandbox_bundle::build_sandbox_task_bundle` derives a
provider-neutral, deterministic export document from an existing valid prepared
task and a **separately reviewed, redacted** `AgentTaskBrief`.

The builder does not automatically export `TASK.md`, the original task title,
the original objective, or the local task root. The reviewed export title and
instructions may intentionally differ from their local versions. A trusted UI
or future control service must display and confirm the exact export scope.

The bundle contains only:

- schema version, task ID and explicitly reviewed export title/instructions
- canonical sorted task-relative `inputs/` paths, declared sizes and SHA-256
- fixed `outputs/` directory and sorted reviewed requested output paths
- bounded input byte total and a versioned, domain-separated metadata digest

It excludes workspace/storage identity, namespace fingerprints, MCP policies,
original source/destination host paths, provider credentials, secret references,
execution records and publication destinations by **structural omission**.

## Limitations and security gates

This pure constructor cannot prove free-form instructions are free of secrets.
It enforces the existing task/brief contract, canonical review text, a 2 GiB
aggregate declared-input limit, a 4 MiB metadata limit and portable input/output
path collision rules. It **does not hash or read the actual input bytes**.

Before any real provider transfer, future C1.2/C2 components must:

1. Obtain explicit human consent to the **exact** reviewed export title,
   instructions, input paths, counts, total bytes and destination provider.
2. Re-resolve task/workspace authority and independently rehash the actual
   prepared bytes under the trusted task boundary. Deny changed or missing
   bytes, links, type drift and path escapes.
3. Stage a bounded, create-only sanitized bundle with no original manifest,
   raw `TASK.md`, local configuration or agent credentials.
4. Upload the staged package, validate the provider's received hash where
   available, download only `outputs/`, and import returned files create-only.
5. Record private execution evidence. Keep existing desktop reviewed
   publication and receipt controls unchanged.

A metadata digest is a **staleness/provenance fingerprint**, not permission
to read source storage, launch remote compute, or publish results.

## Canonical v1 digest vector

The SHA-256 digest is computed over compact UTF-8 JSON of a field-ordered
unsigned envelope with domain
`infimount:v0.8.2:sandbox-task-bundle:v1`, schema version, task ID,
reviewed title/instructions, sorted inputs, fixed outputs directory, sorted
requested outputs and total declared bytes. The digest field is excluded from
its own hash.

The synthetic fixture in `sandbox_bundle.rs` produces **578 canonical bytes**
and the fixed lowercase SHA-256:

`308bdae03cc5e13da6a10dcb5d755d1bb83e8ae614059a92a16949e184a75971`

Tests cover ordering determinism, SHA-256 binding to source metadata and
reviewed instructions, omission of private workspace fields, traversal,
case-collision, invalid controls and excessive declared bytes.

No `infimountctl`, MCP, app/agent, or native user-presence endpoint is added
by this contract. v0.8.1 stable and the v0.8.2 browser-first workbench are
unchanged. See the approved
[governed agent data plane roadmap](roadmaps/v0.8.2-governed-agent-data-plane.md).
