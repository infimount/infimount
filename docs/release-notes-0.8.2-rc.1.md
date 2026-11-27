# Infimount 0.8.2-rc.1: governed artifact integrations

Release: https://github.com/infimount/infimount/releases/tag/v0.8.2-rc.1

This source candidate is prepared for automated qualification. Candidate packages
become available after the canonical Release workflow publishes them. Publication
and validation status are recorded by Release and Post Release Validation.
**v0.8.1 remains the stable release.**

The candidate covers the bounded artifact service and integration scope accepted
in [ADR 0012](adr/0012-hosted-artifact-data-plane.md) and
[ADR 0013](adr/0013-artifact-integrations-and-distribution.md). Other planned work
continues in the [v0.8.2 roadmap](roadmaps/v0.8.2-governed-agent-data-plane.md).

## Desktop MCP binary operations

- `write_file` accepts UTF-8 text or base64 binary content. Limits apply to decoded
  bytes, and invalid or oversized content fails before storage side effects.
- Configurable read and write limits can reduce the existing desktop ceilings.
  Ordinary MCP calls and Agent Task calls use the same configured limits.
- Content types are validated and forwarded when supported by the backend.
  Confirmation requests bind the encoding, content type and exact content.
- Bounded reads preserve the existing response contract and report content
  metadata. Shared path decoding preserves Unicode and rejects malformed escapes.

## Optional hosted artifact service

The separate `infimount-hosted` executable supports one Linux replica with a
dedicated confined filesystem or explicitly configured S3-compatible storage.
The desktop remains independently usable with its existing local configuration.

- Every request verifies an OAuth access token against the configured issuer,
  audience and JWKS. Server-owned grants intersect the verified principal,
  token scopes and existing storage policy.
- The MCP surface provides scoped browsing, metadata, binary reads and create-only
  writes. Uploads use bounded staging, declared sizes and SHA-256 verification.
- Downloads require authentication and use expiring, single-use, owner-bound
  grants over a specific artifact snapshot.
- Concurrency, staging capacity and transfer sizes are bounded. Cancellation
  cleanup retains resource reservations until owned work finishes.
- Structured audit records use fixed operation names and sanitized outcomes.

Deployment remains explicit. Container images and Kubernetes examples do not
create a public service, configure an identity provider or run a live-provider
pilot. See the [hosted service guide](hosted-artifact-service.md) for configuration,
single-replica operation, storage cleanup requirements and deployment boundaries.

## Delegated download links and hosted counters

An optional platform issuer can choose the presentation URL for a download.
Issuance uses a static HTTPS webhook, a dedicated service credential and a pinned
returned origin. Grants remain unavailable until a bounded issuer response is
validated. Failure, timeout or cancellation releases provisional staging and
capacity. Redemption still enforces the original principal, policy, object and
expiry. Direct authenticated proxy delivery remains the default.

An optional OTLP/HTTP JSON exporter sends fixed-cardinality hosted counters to an
explicit HTTPS collector. It exports aggregate operation and byte counts, with
no principal, tenant, client, path, token, grant or artifact labels. Telemetry is
disabled by default.

See [delegated links](delegated-links.md) and
[hosted counters](hosted-artifact-service.md#optional-otlp-export) for the contracts.

## Governed native Python bindings

The `infimount` Python package embeds the shared Rust policy and OpenDAL behavior
inside the Python process. Each client uses explicit local or S3 configuration,
scopes, path policy and bounded byte limits. Binary operations preserve bytes,
and operations requiring human confirmation fail closed.

The release matrix builds native ABI3 wheels for Linux x86_64 and ARM64, macOS
Intel and Apple Silicon, and Windows x86_64. Each wheel must run on CPython 3.10
and 3.14. Linux wheels target glibc 2.28 or later. PyPy, free-threaded CPython,
Linux musl, 32-bit systems and Windows ARM64 are outside this matrix.

Wheels and the source archive are GitHub Release assets. This candidate does not
establish PyPI publication. The library's policy boundary assumes a trusted
embedding application. Python code with the same operating-system authority can
use other libraries or system calls directly. See [Python bindings](python-bindings.md).

## Required automated qualification

The exact candidate source must pass the existing desktop, integration, security,
documentation, rehearsal and release gates, plus these artifact gates:

1. Native Linux amd64 and arm64 hosted container execution with the declared
   non-root user and read-only container root.
2. Hosted identity, isolation, policy, transfer, cancellation and quota regressions,
   including deterministic local filesystem and S3 qualification.
3. Installation and execution of all five native Python wheels on both supported
   test interpreters.
4. A source archive with a qualified dependency lock that rebuilds outside the
   checkout, followed by installed-package and synthetic S3 tests.
5. Exact source, image and package identity checks, checksums, provenance and
   re-download of the draft release before final publication.
6. Automatic Post Release Validation after the candidate is published.

Hosted images use the complete `v0.8.2-rc.1` tag and an immutable digest receipt.
Candidate publication leaves the stable download channel on v0.8.1. A configured
workflow matrix is not evidence that a run has passed. See
[artifact distribution](artifact-distribution.md) for the publication and
verification contract.
