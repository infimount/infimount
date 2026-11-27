# ADR 0013: Complete artifact integrations and distribution

## Status and approval

Accepted for implementation on 2026-11-28. After the maintainer reviewed the
merged ADR 0012 implementation and the explicit remaining list (#249, #251,
#252 and #254), they instructed: "ok lets complete teh rest". This approves
completion of those four engineering tracks, including their qualification
and release packaging, within the v0.8.2 feature train.

The prior Python-binding deferral is superseded for this bounded implementation.
The existing production-deployment and paid-provider-pilot boundaries remain.
No deployment destination, provider credential, spending consent or PyPI account
is inferred from a request to complete repository work.

## Problem and alternatives

The artifact service now has a qualified policy and transfer boundary, but
desktop binary metadata/limits, interoperable telemetry, artifact distribution,
platform-owned link URLs and embedded Python integration remain incomplete.
Finishing these adapters avoids requiring each integrator to recreate storage
policy and capability checks.

Direct provider SDKs or raw OpenDAL Python calls would bypass Infimount's policy
and audit contracts. An authenticated HTTP client remains a supported simpler
integration, but a narrow native binding is now requested for in-process use.
An external link issuer may choose a platform URL without acquiring broader
storage authority. Native desktop links and authenticated proxy links remain
the defaults in their respective entry points.

## Bounded implementation

- Complete configurable desktop MCP read/write limits without raising the
  existing hard ceilings. Apply limits before approval and allocation, preserve
  confirmation binding, and capability-gate binary content metadata.
- Add explicitly configured OTLP export of sanitized bounded telemetry. Keep
  export disabled by default. Never export credentials, access tokens, grant
  URLs, file contents, raw paths or unbounded caller-controlled labels.
- Build and qualify Linux amd64/arm64 hosted images and Python wheels for the
  supported platform matrix. Integrate publication with the canonical release
  gates and immutable source/digest identity. Keep stable and candidate channels
  distinct. Do not assume a separately owned package-registry account.
- Add an opt-in configured delegated link issuer. Pin its HTTPS destination and
  returned URL origin, bound requests/responses/timeouts, preserve expiry and
  replay protection, and retain the original authenticated storage redemption
  requirements. Delegation does not make a link an anonymous bearer credential.
- Add a thin PyO3 binding over shared governed storage operations. Configuration
  is trusted application input, never an untrusted tool argument. Every exposed
  operation applies scope, policy, capability, size and audit checks. There is no
  public raw-operator escape or self-issued human confirmation. Unsupported or
  confirmation-required operations fail closed.

## Browser and authority boundaries

Storage-only onboarding and ordinary desktop browsing remain first-class.
The new integrations do not enable Agent Access, start hosted listeners, export
telemetry or change desktop storage exposure implicitly. Existing native user
publication approval remains separate from all artifact adapters.

In-process bindings cannot isolate hostile Python code running with the same
operating-system authority. Their guarantee is policy enforcement through the
library API. Untrusted agent runtimes continue to use the authenticated service
or an independently confined sandbox.

## Acceptance

Qualify metadata, configured limits and confirmation behavior in both ordinary
and Agent Task MCP dispatch. Prove delegated-issuer failure, timeout, response
validation, owner binding and cleanup with controlled fixtures. Prove disabled
telemetry sends nothing and enabled export contains only the approved schema.
Exercise Python byte round trips and denied policy/capability cases through the
built wheel. Build and test native artifact matrix entries before publication.

The existing nine workflows, desktop release pilot, dependency audit and
cross-platform gates remain required. Add focused integration/distribution
checks. Merge only a qualified source tree. Synthetic tests do not establish
production deployment or paid-provider interoperability.
