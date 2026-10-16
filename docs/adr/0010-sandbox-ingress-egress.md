# 0010 — Remote sandbox ingress and output quarantine

Date: 2026-10-16

## Status

Accepted for v0.8.2 implementation.

## Context

Agent execution providers offer isolated runtimes, filesystems, networks, snapshots, and stateful machines. Infimount does not own their process isolation or retention policy. A sandbox may be operated outside the user's machine, so moving bytes into it is a separate disclosure boundary, not merely a different local agent adapter.

## Decision

1. Remote execution is opt-in with a visible data-disclosure preview: provider, file counts, total bytes, allowed input paths, network posture, data residency/retention caveats, and any runtime persistence choice.
2. Build a separate sanitized bundle from the prepared task. Never send the full local task manifest, source registration, source credentials, native secret-store refs, host paths, destination publication details, or broadly scoped MCP endpoint tokens.
3. Only copy the explicit bounded input files and safe task instructions. Hash the exact local bundle and independently verify upload/download integrity when the provider supports it.
4. Never mount source or destination storage directly into the provider sandbox as an Infimount shortcut. Sandbox execution does not inherit source-system authority.
5. Treat all remote output as hostile/untrusted. Download to private staging, validate archive/container entries before materializing any path, reject absolute paths, `..`, symlinks, reparse points, hardlinks, case collisions, unexpected files, excessive counts/bytes, and non-regular file types. Do not rely solely on filename cleaning.
6. Import approved output entries **create-only** to task `outputs/` and reject existing names. Import is not publication to a source system and does not imply permission to publish later.
7. Record provider, execution ID, sanitized status, bundle digest, output digest, and cleanup outcomes in the private execution record. Omit bearer tokens, raw provider logs, prompts, and private paths.
8. Explicitly terminate or retain provider runtimes according to opt-in policy. Warm pools and snapshots are performance mechanisms, not isolation credentials.
9. Make cancellation, partial import, cloud/network failure, and provider runtime cleanup recoverable and observable. Do not claim rollback when a backend cannot guarantee it.
10. Do not claim that Infimount can prevent the external provider from retaining an uploaded copy. The boundary is controlled disclosure plus governed local write-back, not remote erasure guarantees.

## Rejected alternatives

- Mount original S3/Drive/local source into remote sandboxes.
- Reuse one runtime across tasks without clearing prior task data.
- Directly publish sandbox artifacts to source/destination storages.
- Trust a provider's filename metadata or output archive contents.
- Call a sandbox snapshot an Infimount audit/provenance receipt.

## Consequences and tests

C1 establishes provider-independent sanitizer/import tests. C2 proves the first external sandbox. Reject malicious archives, path collisions, oversize outputs, mutation of source digests, leaked credentials, canceled transfers, re-used outputs, and expired provider resources. Require real provider regression before advertising a sandbox as supported.
