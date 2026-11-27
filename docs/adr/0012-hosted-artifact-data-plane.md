# ADR 0012: Optional hosted artifact data plane

## Status and approval

Accepted for implementation on 2026-11-27. The maintainer explicitly accepted
the proposed scope with "yes lets complete all of this" after reviewing the
bounded contract in [decision request #248](https://github.com/infimount/infimount/issues/248#issuecomment-7223867189).
This records the product decision required by PRODUCT.md. It does not authorize
a public deployment, paid provider requests, or the separate live-provider pilot.

## Problem and alternatives

Agent platforms need to save binary artifacts into several storage backends and
return authorized downloads to the requesting user. A desktop sidecar's shared
token and local sessions do not establish isolation for a multi-user service.

A thin platform-specific file service is appropriate for one application, but
duplicates storage capabilities and policy integration across applications.
Provider presigned URLs are simpler for one object store but cannot uniformly
serve a confined filesystem, keep backend endpoints private, or recheck identity
on every download. Isolated per-tenant deployments reduce shared-service risk at
the cost of deployment and credential overhead. An authentication proxy alone
cannot enforce object-level authorization. This decision adds a bounded opt-in
service and retains those deployment alternatives.

## Scope and product boundary

- Continue the v0.8.2 feature train. Do not change the existing release version.
- Add a separate `infimount-hosted` entry point, explicitly started with trusted
  configuration. The desktop remains a local-first storage browser with optional
  Agent Access and Agent Tasks. No new desktop setup or network requirement.
- Use OpenDAL for storage byte operations, with shared core code and the existing
  storage policy evaluator. Do not construct a mutable desktop registry as the
  hosted authority.
- Begin with one replica, S3-compatible storage and a dedicated confined local
  filesystem. A local backend must fail closed when confinement cannot be
  provided. Do not claim that an OpenDAL root or a string prefix contains links.
- Support list, stat, bounded binary read and create-only write, authenticated
  downloads, and bounded staged streaming uploads. The absence of a backend's
  atomic create capability is a refusal, never an overwrite fallback.
- Exclude deletion, moves, overwrites, source publication, administration,
  cross-user sharing, collaboration, an agent runtime and a generic MCP gateway.
  Python bindings remain deferred. A thin authenticated client is the initial
  Python integration boundary.

## Identity and authorization

Every protected request verifies an asymmetric OAuth access token against a
pinned issuer, audience, algorithm and bounded JWKS source. Reject unsigned
tokens, ID tokens, foreign audiences, unrecognized critical headers and expired
authority. Never trust a caller-supplied filesystem root or identity header.

An immutable request-local principal contains issuer, subject, tenant, client,
scopes and expiry. An explicit server-owned mapping grants canonical storage
prefixes. Effective permission is the intersection of that mapping, token scopes
and existing storage policy. A policy requiring confirmation is denied because
this service has no self-approval path.

Use stateless MCP HTTP. Do not expose application sessions or accept transport
session IDs. Bind opaque cursors and transfer grants to the full principal,
operation, object, storage/config revision and expiry. Recheck authorization on
each use. A restart invalidates in-memory grants. No HA claim is made until state,
quotas, replay protection and revocation are coordinated across replicas.

## Downloads, writes and streaming

Downloads use short-lived opaque server-owned grants and require the same
authenticated principal at redemption. Grants bind an immutable version or a
verified object identity, and are unusable after relevant changes. No backend
redirect, credential, endpoint or transferable bearer link is returned. Response
headers force a safe attachment, octet-stream content, no sniffing and private
no-store caching. Platforms relay downloads through their authenticated browser
integration. Anonymous sharing and delegated links require a separate decision.

Large bytes travel on a separate authenticated HTTP data path. MCP carries
metadata and grants. Admission bounds JSON before parsing. Transfers reserve
bounded capacity, enforce declared size and checksum, stream in bounded chunks,
expire, and support cancellation and cleanup. A completed upload publishes only
after validation using atomic create-only semantics. Failed, replayed or
conflicting operations must not expose partial destination bytes.

## Deployment and operations

Configuration and credentials come from explicit files and environment/secret
references. Run as non-root with a read-only root filesystem and dedicated data
mounts. TLS is terminated by trusted ingress, with network policy preventing a
bypass. Pin public Host/Origin values. Do not use forwarded identity headers.
Apply request, byte, concurrency, state-count and expiry limits. Emit sanitized
structured audit to the operator's log collector, including verified identity
and allowed/denied result, without tokens, secrets, signed URLs or file contents.
Liveness is cheap. Readiness is bounded and cannot become an unbounded storage
probe on each request. No deployment is created by implementation or CI.

## Acceptance and regression gates

- Two users and two tenants cannot discover or operate on each other's objects,
  cursors, uploads or downloads, including altered, expired and replayed grants.
- Wrong issuer/audience/algorithm, malformed claims, ID tokens, missing scopes,
  unsafe paths, missing principal and JWKS failures fail closed.
- Exact generated binary bytes can be saved, read and downloaded through both
  qualified backends. Conflicting creates preserve existing bytes. Symlink
  escapes and unsafe filesystem objects are rejected without races.
- Size, checksum, concurrency, disconnect, abort, restart and policy/revision
  changes fail safely, with bounded state and cleanup.
- The existing eight desktop/repository workflows remain merge gates. Add
  focused hosted tests, container build validation and synthetic S3 qualification
  without paid credentials. Browser regressions block release.

Implementation evidence belongs in the PR. Passing synthetic tests is not live
provider qualification or production deployment approval.

