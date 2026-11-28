# Artifact acceptance and release review

## Decision and scope

Release remains on hold at the maintainer's request. This review checks the
requirements in [#248](https://github.com/infimount/infimount/issues/248) and
[#249](https://github.com/infimount/infimount/issues/249) through
[#254](https://github.com/infimount/infimount/issues/254) against the accepted
[ADR 0012](adr/0012-hosted-artifact-data-plane.md) and
[ADR 0013](adr/0013-artifact-integrations-and-distribution.md).

The reviewed baseline is `508c3d3fc93dbc1815bab3ed31c8282eee39ce94`, the merge of
[#261](https://github.com/infimount/infimount/pull/261). Its existing green
workflows did not establish all the requested scenarios. The corrections and
additional tests accompanying this report require their own exact-source
qualification. No candidate tag, release, image publication, deployment or
paid-provider pilot is performed by this review.

The principal acceptance scenario is concrete: an agent creates a binary
artifact, its authorized user retrieves the same bytes, and a different user
or tenant cannot access the artifact through another entry point.

## Findings and corrections

| Finding | Impact and evidence | Correction and regression |
| --- | --- | --- |
| List cursors disclosed policy-hidden filenames | The core cursor authenticated base64 JSON but did not encrypt it. A denied lookahead entry could appear in its decoded `position`. Reproduced through the exact previously qualified Python wheel. Desktop MCP forwarded the same cursor. Hosted HTTP already wrapped backend cursors in opaque server state. | Encrypt list continuation state with XChaCha20-Poly1305. Bind path, recursive mode and revision as authenticated data. Test denied lookahead, scan-budget boundaries, tampering, old cursors and cross-query replay. |
| Agent Task writes inherited an overwrite default | A task write with omitted `overwrite` inherited the ordinary filesystem handler's `true` default. A native task handoff authorized new outputs, so this could replace an existing output without the required destructive approval. | Default task writes to `false`, reject explicit overwrite and supplied confirmation authority, and advertise the create-only schema. Test omitted/explicit arguments, preserved bytes, the native create approval and later policy denial. |
| Desktop approval could follow a changed storage target | Approval fingerprints covered arguments and byte limits, but did not bind storage identity, namespace and policy. An approved request could be replayed after a storage was retargeted. | Bind every participating storage's identity, namespace, revision and policy. Hold the configuration transaction through execution. Test changed roots, identities, policies, a removed confirmation requirement and a changed copy source. |
| Transfer expiry could exceed the issuing token deadline | Proxy and upload grants lacked the absolute expiry check used by delegated grants. Rounded seconds could allow a refreshed same-owner token to redeem after the issuing token's deadline. Proxy receipts also retained the pre-snapshot TTL. Both new regressions fail on the original implementation. | Give every grant an absolute deadline, derive the monotonic deadline from the remaining exact duration, cap commit authority and report remaining lifetime. Test PUT, completion, proxy and delegated redemption plus quota cleanup. |
| Desktop preview trusted the backend range response | The older preview helper collected a ranged response with `read_to_end`. A provider returning the whole object could exceed the requested bound. | Use the shared bounded range collector. A synthetic backend deliberately ignores Range and the regression checks rejection after a bounded chunk. |
| Python cursor size limits did not agree | A valid long-path listing could produce a cursor larger than the Python API's accepted 4 KiB. Reproduced through the previously qualified wheel. | Avoid duplicating the query path in the encrypted cursor and use the shared 16 KiB ceiling. Test traversal of all pages under a long Linux path. |
| Python reserved namespace handling was inconsistent | The current private `.infimount-hosted-staging` directory was excluded, but the additional `.infimount-hosted-stage` spelling reserved by the hosted path contract was accepted. This is a consistency defect, not proof of exposure of the current private staging directory. | Reserve both names. Test root-list filtering, denied read/stat/list/write and absence of side effects. |
| Canceled filesystem work released capacity too early | Two blocked-filesystem regressions fail on the original implementation because queued work outlives its pinned descriptor and capacity reservation. | Retain descriptor and resource ownership until the accepted operation finishes. Use bounded owner queues and dedicated threads so a one-worker Tokio pool also completes. Cover read/stat/list, download bodies, staged publication and single-flight readiness. |
| Decoded file limits did not bound raw request collection | The SDK collected HTTP bodies and stdio lines before tool preflight. Decoded size checks alone did not bound that earlier allocation. | Bound authenticated HTTP bodies and stdio frames before JSON parsing, allowing worst-case escaped valid payloads. Test chunked HTTP rejection, authentication-before-body, per-frame reset and existing maximum-size artifact round trips. |
| Native container qualification stopped at configuration parsing | The old job ran `check` with networking disabled. That command intentionally does not start the listener, resolve credentials, verify OAuth or access storage. | Run the exact native image through the production-process acceptance harness before preserving its release archive. Keep the offline configuration check as a separate assertion. |
| Byte-limit visual acceptance left its scroll position undefined | A final CI run passed the saved-value and disabled-Agent-Access assertions, then failed its screenshot comparison. The captured image showed the same correct state with the dialog scrolled roughly 284 pixels below the baseline. | Explicitly frame and verify the top of the scrollable dialog before both captures. Preserve the original visual baseline and every functional assertion. |
| Post-release tag input entered shell source before validation | Event and manual tag values were directly interpolated into a shell script before the SemVer check. | Pass values as environment data and validate the selected value before using it. |

## Requirements and scenario coverage

| Requirement | Expected scenarios | Evidence in this change and retained gates | Accepted boundary |
| --- | --- | --- | --- |
| Binary MCP I/O, #249 | Exact bytes, base64 validation, configured limits, collisions, MIME, approval and audit in ordinary and Agent Task dispatch | `crates/mcp/src/server.rs`, `agent_task_server.rs`, typed filesystem tests, native desktop and release-pilot gates | Existing hard ceilings remain. Unsupported MIME capabilities fail before creating a file. |
| Request-local identity, #250 | Forged/expired/wrong issuer/wrong audience tokens, tenant/client substitutions, missing scope, JWKS rotation and stale-cache failure | `auth.rs` and `transport.rs` regressions, plus real signed OAuth tokens and a TLS JWKS endpoint in the process harness | Static server-owned grants intersect token scopes and policy. No claim-to-path interpolation, arbitrary delegation headers or ambient sessions. |
| Every entry point preserves isolation | Read, list, stat, create, upload PUT, complete, abort, download, cursors and replay | Process harness covers two principals and two tenants on both local and S3 storage. Unit tests cover malformed headers, unpolled unauthenticated bodies and SDK bypass attempts. | Hosted destructive/admin operations are absent. Their absence is asserted, not treated as missing functionality. |
| Headless service, #251 | Explicit configuration, independent process, real startup, health/readiness, secret references and non-root packaging | Production binary harness, native amd64/arm64 image harness, strict config tests and Kubernetes examples. The process harness stops and restores its S3 endpoint, requiring bounded readiness failure/recovery, unchanged liveness and preserved files in the same running service. | One Linux replica. Ingress and external identity-provider provisioning remain deployment responsibilities. |
| Telemetry, #251 | Disabled mode ignores ambient OTEL configuration, enabled mode reaches a configured TLS collector, bounded fields and redaction | `telemetry.rs` and `metrics.rs` unit fixtures plus process-level OTLP collection and production audit-schema assertions | Fixed-cardinality metrics only. Audit identities are intentional audit fields and are not metric labels. |
| Artifact delivery, #252 | Authorized byte equality, foreign-owner denial, expiry, single use, safe attachment headers, native and delegated modes | Transfer tests and process harness. Delegated presentation links use a separate TLS platform origin with its own session cookie and a server-side OAuth relay. Wrong/missing platform sessions fail. | Hosted links require the original identity. They are not anonymous share links. Native provider links retain their separately documented bearer-link semantics. |
| Large transfers, #253 | Large binary upload, exact size/SHA-256, atomic create-only completion, ranged reads, abort, checksum failure, quota recovery, cancellation and restart | Transfer and confined-I/O tests, Moto S3 multipart qualification, production 9 MiB ZIP upload/download, competing uploads above 9 MiB and actual listener disconnects. S3 completion headers, conflict status, winning bytes and unfinished-upload cleanup are checked explicitly. | One authenticated PUT plus explicit completion. Resumable uploads and multiple replicas are outside the accepted implementation. The conditional-write fixture models the backend's atomic primitive, as detailed below. |
| Governed Python API, #254 | Installed-wheel byte round trips, create-only writes, limits, scopes, policy, confirmation refusal, path aliases, reserved internals, audit and native S3 links | `python/tests/test_native.py`, `python/qualify_s3.py`, five native wheel targets tested on CPython 3.10 and 3.14, isolated source-archive rebuild | Trusted embedding application selects immutable configuration. The library does not isolate hostile same-process Python. |
| Existing browser behavior | Storage-only onboarding, explicit Agent Access, browsing/preview, native approval and packaged sidecar behavior | Existing desktop CI, coverage, integration, hosted-pilot, release-pilot and installer-upgrade workflows remain required | No automatic hosted listener, storage exposure or telemetry activation is introduced. |
| Release integrity | Exact source identity, native artifacts, inventory, checksums, provenance, immutable image promotion and post-release verification | Existing release/distribution scripts and their regression fixtures remain gates. The stronger image process test runs before archiving that image. | No claim of publication is made from a successful source build or configuration check. |

## Production-process acceptance harness

`scripts/qualify-hosted-process.py` supports either `--binary` or `--image`.
It creates a temporary CA and verifies real HTTPS certificates, generates RSA
access tokens, starts the actual `serve --config` executable, and routes MCP and
artifact traffic through a local TLS ingress fixture. It does not use a
test-only authenticator or disable certificate validation.

Each backend exercises eight scenario groups:

1. Startup, health, metadata, exact tool allowlist and OAuth rejection cases.
2. PNG, SVG, CSV, PDF and XLSX bytes plus a ZIP larger than the inline limit.
   S3 metadata is checked with `head_object`, and downloads are compared byte
   for byte. Repeated writes and link redemption must fail.
3. Cross-tenant access through every applicable tool, cursor/grant substitution,
   traversal, a non-dangling symlink escape, unsupported authority arguments and
   configured body/inline limits. The oversized-header test talks directly to
   the real upstream listener and requires rejection before sending any body.
4. Incorrect checksums, abort, a small-object completion race, staging quota
   exhaustion/recovery, expiry and a single-use byte-range download.
5. Two different uploads above 9 MiB compete for the same destination. Exactly
   one succeeds, the other returns a conflict, and an authenticated download
   preserves every winning byte. S3 assertions verify the actual conditional
   completion requests and absence of unfinished multipart uploads.
6. Partial PUT and large GET clients disconnect from the actual listener. The
   PUT must first receive `100 Continue`. The GET's small receive window and
   quota assertion prove that the response is still in flight. Full staging
   quota must recover before the original grant expires, partial uploads must
   leave no destination, and both grants must reject replay.
7. Live JWKS rotation, disabled/enabled telemetry, invalidated grants after
   restart, preserved committed files and a restarted policy revocation.
8. Delegated issuer failure and wrong-origin cleanup, a working platform login
   relay, single-use redemption, exact bytes and production audit redaction.

The S3 backend adds a ninth group: stop the dependency, observe readiness change
from 200 to 503 within a bound while liveness remains 200, restore the same
endpoint and verify readiness and stored bytes recover without restarting the
service. Together these are 17 process scenario groups.

Image mode uses the existing image's non-root user, read-only root, dropped
capabilities and no-new-privileges. Host networking connects only to disposable
loopback fixtures. Temporary fixture storage is the only writable host mount.
The same UID removes its private mode-0700 staging directory before host cleanup.
This is a native packaged-process acceptance test, not a Kubernetes or external
provider deployment test.

### Conditional multipart evidence

The added large-upload race exposed a limitation in threaded Moto 5.2.6. Two
same-key multipart completions both succeeded. An independent direct-boto3
reproducer, with Infimount and OpenDAL removed, returned two HTTP 200 responses
in all three concurrent trials despite both requests carrying `If-None-Match:
*`. Its sequential control returned 200 followed by 412. Moto checks for the
existing object before assembling the parts and publishing it, without making
that check and publication atomic. Its [official FAQ](https://docs.getmoto.org/en/latest/docs/faq.html)
states that concurrent access is not supported.

This differs from [Amazon S3's conditional-write contract](https://docs.aws.amazon.com/AmazonS3/latest/userguide/conditional-writes.html):
the first conditional write to finish succeeds and later competing writes fail.
The locked OpenDAL S3 implementation supplies the required condition on
`CompleteMultipartUpload`. A simulator race is not evidence that Amazon S3 has
the same defect, and it must not be concealed by changing production writes or
accepting two winners. Live-provider concurrency remains a separate qualification
boundary.

The process fixture therefore serves Moto through its public WSGI application
with one request handler at a time. This supplies the backend atomicity that
the conditional-write protocol requires. Two production completion requests
still compete. The harness records two distinct multipart IDs, verifies both
actual `If-None-Match` headers, requires HTTP 200 and 412, checks the exact winner
and confirms loser cleanup. Independent fixture controls require two
unconditional completions to overwrite successfully and the second conditional
completion to fail. Removing the production precondition would still fail the
acceptance test. This is protocol qualification against a serialized synthetic
backend, not live-provider concurrent-commit qualification.

## Evidence interpretation and release gates

The review reruns the existing Rust suites and the distribution, source archive,
image-publisher and release-preparation script tests. New regressions are also
run against the old behavior where practical so a green result cannot come from
an assertion that never reaches the failing condition. The downloaded old
Python wheel exposes both the hidden-name and long-cursor failures. The old
transfer implementation fails both new expiry regressions.

Release readiness requires the corrected source to pass all existing required
workflows, the stronger native-image tests and the installed-wheel matrix.
Source-level success on one Linux machine does not replace native macOS,
Windows or ARM64 evidence. GitHub check runs on the review PR provide the exact
commit and runner evidence.

## Items that are not yet demonstrated downstream

The original request described a Deep Agents or sandbox `/outputs/` backend.
The accepted implementation supplies a governed native `Client` and an
authenticated hosted protocol. It does not supply or qualify a drop-in Deep
Agents backend adapter. Its byte API, create-only writes and storage-qualified
paths need an explicit adapter contract if that runtime integration is part of
the consuming application's launch. The report must not label that integration
complete merely because the native wheel imports successfully.

No real external identity provider, production ingress, Kubernetes cluster or
paid S3-compatible provider is exercised here. Moto proves the controlled S3
protocol scenarios only. Configuration, failure and policy tests are not
performance or availability qualification.

The prior release-preparation run also failed with HTTP 404 in its write-capable
job. Its logs omit the failing endpoint, so the exact cause remains unverified.
The job did receive contents/actions write permission. Independently, the
repository restricts `v*` tag creation and the workflow token has no configured
bypass. This review does not weaken that rule or retry publication. The
publication route must be reconciled with repository policy after the code and
scenario review is accepted.
