# v0.8.2 C1.2: remote sandbox output import validator

Status: **pure, provider-neutral candidate validator**, no upload, download,
archive extraction, local file write, execution or publication authority.

## Threat boundary

A remote sandbox or compromised provider can return arbitrary listing metadata.
It may lie about a file's type, byte size, SHA-256, path or contents.
Infimount must not treat a provider's JSON as evidence that it downloaded
a safe, regular file, nor as authorization to import it or publish it.

`infimount_control::sandbox_import` provides two functions:

- `parse_and_validate_sandbox_output_listing` rejects remote JSON exceeding
  4 MiB, unknown fields, unsupported versions and unrecognized file types
  before any local mutation.
- `validate_sandbox_output_listing` returns a canonical, sorted, bounded
  **candidate plan** with a domain-separated metadata SHA-256. That fingerprint
  covers reported metadata, **not** verified bytes and not human consent.

## C1 first-cut policy

- Accept nonempty v1 listings with at most 1,000 regular-file claims and a
  combined declared size at most 2 GiB, with checked arithmetic.
- Accept only canonical paths under `outputs/`, with no traversal,
  backslashes, control characters, absolute names, ASCII case-collisions,
  file/ancestor conflicts, percent-encoded ambiguity, Windows device names
  or trailing dot/space.
- Use the ASCII portable filename intersection for this first cut, since
  Unicode NFC/NFD/casefold differences across Windows/macOS and providers
  are not yet qualified. Future support can widen this contract with tests.
- Reject directory, symlink and special-file claims. Empty directories are
  not imported. Reject extra JSON authority fields, rather than ignoring them.
- Never accept the remote listing metadata digest as evidence of file integrity.

## Deterministic v1 test vector

The sorted, domain-separated candidate listing used by the control-crate
regression test encodes to **362 compact UTF-8 JSON bytes** and hashes to:

`1617e491c7145b61eb7c11cd4e6c497c6b8f379fa75c6a0cda3ad4785af96f8e`

This is only a fingerprint of **provider-declared metadata**, not proof of the
downloaded bytes, signed provider identity, approved contents or permission
to commit. Rehash actual files under independent local controls before import.

## Requirements BEFORE any C1.3 real importer

1. The trusted local task owner must validate the destination workspace/task
   identity under the same workspace mutation lock as the actual create-only
   copy. The sandbox must not choose a destination directory.
2. Re-enumerate the actual downloaded output tree with no-follow file-type
   checks, including symlinks/reparse points and parent path components.
   A claimed `kind: file` value cannot replace an OS-backed type check.
3. Rehash every downloaded file's bytes and compare exact size and SHA-256 to
   its validated candidate, before and during streaming.
4. Copy using the existing create-only verified writer, refuse any existing
   output destination, and keep integrity-checked receipts/evidence private.
   On partial commits, report explicit cleanup required and never remove
   destinations that a concurrent writer may have replaced.
5. Revalidate total file and byte bounds after actual byte hashing. Require
   explicit user review of outputs before **publication** into original storage.
6. Record sanitized provider failures and execution state in the existing
   private execution record. Do not store raw credentials or host paths in
   a provider-visible task bundle.

A future importer must preserve ordinary local/cloud browsing without agent
setup and the desktop-mediated publication contract. No headless approval
or MCP model-callable final publication endpoint is enabled here.

See the [v0.8.2 governed agent data-plane roadmap](roadmaps/v0.8.2-governed-agent-data-plane.md)
and [sanitized sandbox task bundle](sandbox-task-bundle.md) for the outbound
contract. This validator is independently usable until the outbound C1.1
contract merges.
