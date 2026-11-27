# Native Python storage integration

Release status: included in the [v0.8.2-rc.1 source candidate](release-notes-0.8.2-rc.1.md),
prepared for automated qualification. Candidate packages become available after
release publication. v0.8.1 remains the stable release.

The `infimount` Python package embeds a narrow Rust adapter over Infimount's
shared policy, path validation and OpenDAL operations. It works in the Python
process and does not start the desktop application, an MCP listener or the
hosted Linux service. The implementation is approved by
[ADR 0013](adr/0013-artifact-integrations-and-distribution.md).

## Authority and configuration

Construct each `Client` from configuration owned by the trusted embedding
application. The constructor selects one storage, explicit scopes, an immutable
MCP v2 path policy and limits. Never pass an agent's proposed configuration to
the constructor. A caller that can choose its own root, credentials and policy
can choose its own storage authority.

Every operation applies the configured scope, shared MCP policy and backend
capabilities. `RequireConfirmation` raises `ConfirmationRequired`. There is no
confirmation token, automatic approval callback or method for granting an
operation to itself. The configuration may explicitly allow unattended work
within a selected prefix. That is a trusted application decision made before
the client is given to a caller.

The module does not load desktop storage records, keyring credentials, Agent
Access sessions or ambient AWS credentials. It exposes no raw OpenDAL
operator. The existing local MCP guard also protects Infimount's execution
control directory. Policy configuration uses manual rules, not desktop
workspace identities.

This library enforces governance through its API. Python code with the same
operating-system authority can bypass any in-process library using another
library or a system call. Use the authenticated hosted service or an independent
sandbox when the Python code itself is untrusted.

## Install and platform qualification

Release wheels and the source archive are attached to the canonical GitHub
release. Download the wheel matching the operating system and architecture,
verify its release checksum, and install that local file with `python -m pip
install`. No PyPI publication or ownership is implied by this integration.
See [artifact distribution](artifact-distribution.md) for channel and inventory
rules.

The package uses [PyO3 0.30.1](https://docs.rs/pyo3/0.30.1/pyo3/) and
[Maturin 1.16.0](https://www.maturin.rs/changelog.html#1160). It selects
`abi3-py310` for standard GIL-enabled CPython 3.10 and later. The
[PyO3 build guide](https://pyo3.rs/v0.30.1/building-and-distribution.html)
explains the stable ABI and why a standard `abi3` wheel does not qualify a
free-threaded interpreter. PyPy, free-threaded CPython, 32-bit systems, Linux
musl and Windows ARM64 are outside this release matrix.

| Wheel platform | Native build target | Required installed-wheel tests |
| --- | --- | --- |
| Linux x86_64 | `x86_64-unknown-linux-gnu`, manylinux 2.28 | CPython 3.10 and 3.14 |
| Linux ARM64 | `aarch64-unknown-linux-gnu`, manylinux 2.28 | CPython 3.10 and 3.14 |
| macOS Intel | `x86_64-apple-darwin` | CPython 3.10 and 3.14 |
| macOS Apple Silicon | `aarch64-apple-darwin` | CPython 3.10 and 3.14 |
| Windows x86_64 | `x86_64-pc-windows-msvc` | CPython 3.10 and 3.14 |

The workflow builds each architecture natively, installs the same wheel on
both Python versions, and checks the complete distribution inventory before
release. A configured matrix is not evidence of a passing run. A release is
qualified only when its exact source tree passes these jobs and the existing
release gates. Linux wheels require a glibc 2.28-compatible environment. Consult
the actual macOS wheel tag for its minimum deployment target.

## Local storage example

The following complete example creates a temporary, application-owned root,
permits reads and new files under `outputs`, and excludes `outputs/private`.
The trusted application prepares the directories. The API does not create
parent directories implicitly.

```python
from pathlib import Path
from tempfile import TemporaryDirectory

from infimount import AlreadyExists, Client

with TemporaryDirectory(prefix="infimount-example-") as directory:
    root = Path(directory).resolve()
    (root / "outputs").mkdir()
    (root / "outputs/private").mkdir()
    config = {
        "storage": {
            "backend": "local",
            "name": "Artifacts",
            "root": str(root),
            "read_only": False,
        },
        "scopes": ["infimount:read", "infimount:create"],
        "policy": {
            "version": 2,
            "default_access": "none",
            "rules": [{
                "id": "application-outputs",
                "prefix": "outputs",
                "access": "read_write",
                "source": {"kind": "manual"},
                "confirmation_rules": {"require_for_write": False},
            }],
            "denied_paths": ["outputs/private"],
        },
        "limits": {
            "read_bytes": 2 * 1024 * 1024,
            "write_bytes": 4 * 1024 * 1024,
            "concurrent_operations": 4,
            "io_timeout_seconds": 30,
        },
    }
    with Client.from_config(config) as client:
        path = "/Artifacts/outputs/example.bin"
        payload = b"\x00\x89PNG\xff" + bytes(range(256))
        assert client.write(path, payload) == len(payload)
        assert client.read(path) == payload
        assert client.stat(path)["size"] == len(payload)
        try:
            client.write(path, b"replacement")
        except AlreadyExists:
            pass
        else:
            raise AssertionError("Create-only writes must preserve an existing file")
        assert client.read(path) == payload
        print(client.list("/Artifacts/outputs"))
        print(client.drain_audit_events())
```

Root paths must be existing, absolute directories. Resolve system aliases such
as macOS `/var` or `/tmp` before configuring the root. The root and its ancestors
must not be symbolic links or Windows reparse points. Local request paths reject
symbolic links, reparse points, traversal, alternate data stream syntax and
ambiguous trailing dots or spaces. They must use the spelling stored in the
directory. Case or Unicode decomposition aliases of grants and exclusions are
denied, including aliases of exclusions that do not exist yet. Each component's
spelling check scans at most 16,384 directory names and fails if that bound is
exhausted.

These portable checks assume the application owns the configured root and
prevents hostile concurrent changes to its directories. They do not provide
the hosted Linux service's descriptor-confined filesystem boundary. Do not use
a shared, attacker-writable root as an isolation boundary. Hard links and
mounts intentionally present inside a trusted root also remain application
configuration concerns.

## Explicit S3 configuration

S3 uses the same scopes and policy. Replace only `config["storage"]` with a
trusted deployment configuration such as this:

```python
import os

config["storage"] = {
    "backend": "s3",
    "name": "Artifacts",
    "bucket": os.environ["APP_ARTIFACT_BUCKET"],
    "region": os.environ["APP_ARTIFACT_REGION"],
    "endpoint": os.environ["APP_ARTIFACT_ENDPOINT"],
    "access_key_id": {"kind": "env", "name": "APP_ARTIFACT_ACCESS_KEY_ID"},
    "secret_access_key": {"kind": "env", "name": "APP_ARTIFACT_SECRET_ACCESS_KEY"},
    "read_only": False,
}
```

The application must supply those environment variables before constructing
the client. The credential references are explicit. Missing references do not
fall back to `AWS_ACCESS_KEY_ID`, profiles, instance metadata or the desktop
keyring. A credential can instead use `{"kind": "file", "path": absolute_path}`
for a trusted, regular UTF-8 secret file. Terminal CR/LF characters are trimmed.
Secret input is bounded to 64 KiB. Expanded credentials are never returned from
the client or retained in its public storage record.

An S3 endpoint must use HTTPS. HTTP is accepted only for an explicit loopback
endpoint used in controlled local qualification. User information, query
strings and fragments are rejected. The narrow constructor does not support
ambient role discovery, session tokens, arbitrary backend configuration,
default ACLs or silently added key prefixes. Additional S3-compatible providers
must demonstrate the requested capabilities. A provider advertising support
but rejecting a conditional create fails the operation. The adapter never
falls back to a preceding existence check followed by an unconditional write.

## API and operation semantics

All paths use `/StorageName/relative/path`. Policy prefixes are relative to the
configured backend root or bucket. Scopes are explicit strings. Missing scopes
and a missing policy default to denial. Unknown configuration fields are
rejected, including invented approval flags. Policy paths with leading or
trailing whitespace are rejected instead of being silently reinterpreted.

| Method | Required scopes | Policy and result |
| --- | --- | --- |
| `list(path, limit=200, cursor=None)` | `infimount:read` | `List` permission. One filtered, nonrecursive page with `entries`, `next_cursor`, `truncated`. |
| `stat(path)` | `infimount:read` | `Metadata` permission. Returns `path`, `name`, `is_dir`, `size`, `modified_at`, `etag`. |
| `read(path, offset=0, max_bytes=None)` | `infimount:read` | `Read` permission. Returns `bytes`, including arbitrary binary data. |
| `write(path, data)` | `infimount:create` | `Write` must be allowed without confirmation. Accepts `bytes` and returns the created byte count. |
| `delete(path)` | `infimount:delete` | `Delete` must be allowed without confirmation. Deletes one file and returns `None`. |
| `download_link(path, expires_seconds=300)` | `infimount:read` and `infimount:download` | Both `Read` and `PresignDownloadLink` must be allowed. Returns a native presigned GET URL, required headers and TTL. |
| `drain_audit_events()` | Client ownership | Returns bounded sanitized events and a dropped-event count. |
| `close()` | Client ownership | Prevents new calls. Calls already running finish normally. |

List pages contain at most 200 entries. Hidden children are filtered before
their metadata is fetched. Continue while `next_cursor` is present, even if a
page is empty. Cursors are signed and bound to this client's immutable revision,
path and listing mode. They cannot move between clients. If a page has
`truncated: true` and no continuation cursor, the backend scan bound was reached
and the result is incomplete. Directory order and concurrent changes can affect
pages. No lexical ordering or snapshot listing is promised.

Omitting `max_bytes` requires the complete file remainder to fit the read limit.
An oversized whole-file request raises `LimitExceeded` instead of silently
returning a prefix. Supplying `max_bytes` requests a bounded range. Reading at
EOF returns `b""`. An offset beyond EOF is invalid. Range reads do not promise
a consistent snapshot across separate requests while an object changes.

Writes require the backend's conditional create capability. Existing targets
raise `AlreadyExists` and remain unchanged. Empty byte files are valid. Local
parent directories must already exist. There is no overwrite, append, metadata
mutation, automatic parent creation or transaction API. A provider failure or
timeout can leave an ambiguous or partially written new object. Inspect the
target before choosing recovery. Do not assume a failed call guarantees rollback.

Deletion is never recursive and rejects directories. Both
`require_for_delete` and `require_for_write` must be false in the effective
trusted rule for an unattended delete. The library does not consume desktop
approval IDs. `read_only: true` independently blocks write-like operations.
An S3 prefix containing objects need not have a materialized directory object.
Listing that prefix can succeed while stat or delete of the bare prefix returns
`NotFound`. Its child objects remain untouched.

Native download links require `require_for_presign: false` in the effective
rule. Their lifetime must be 60 through 900 seconds. Local files do not support
presigning. A successful S3 result contains `url`, `method: "GET"`, `headers`
and `expires_seconds`. Send the returned headers with the GET request. The URL
is an intentionally issued provider bearer credential and must be protected
accordingly. It is distinct from the hosted service's authenticated download
route or delegated authenticated link. Native presigning does not create a
session, proxy server or public bucket ACL.

## Limits, execution and audit

| Limit | Default | Hard maximum |
| --- | --- | --- |
| Bytes returned by one read | 2 MiB | 2 MiB |
| Bytes submitted in one write | 4 MiB | 4 MiB |
| Concurrent operations per client | 4 | 16 |
| S3 I/O timeout | 30 seconds | 60 seconds |
| Entries requested per list page | 200 | 200 |
| In-memory audit events | 256 | 256 |
| Trusted configuration JSON | 1 MiB | 1 MiB |
| Rules or exclusions | 128 each | 128 each |

Read/write limits can only be reduced. An excessive Python byte buffer is
rejected before copying it into Rust. The concurrency limit rejects excess
calls immediately with `LimitExceeded`. These limits are per client. The trusted
embedding application controls how many clients it creates.

Calls are synchronous and release the GIL while performing native storage work.
An async application can use `await asyncio.to_thread(client.read, path)`.
Cancelling that await does not cancel the native operation. Local filesystem
I/O is allowed to finish with its concurrency permit held because dropping a
Tokio future does not cancel queued operating-system file work. The I/O timeout
applies to remote S3 operations, not local filesystem calls. No immediate
cancellation or hard wall-clock deadline is promised for an entire multi-step
operation. Recreate clients after `fork`. Inherited clients reject use before
touching their runtime or locks. Clients cannot be pickled.

Audit uses the shared MCP event shape with actor `python`, a random storage ID,
fixed operation and decision names, elapsed time, successful byte counts and a
SHA-256 path digest. It excludes raw paths, credentials, object contents,
presigned URLs and ambient activation-probe identity. Errors expose fixed codes
through typed exceptions such as `PermissionDenied`, `ConfirmationRequired`,
`AlreadyExists`, `LimitExceeded` and `BackendUnavailable`. Python argument type
errors remain ordinary Python exceptions.

Each client retains at most 256 events. New events replace the oldest and
increment `dropped`. `drain_audit_events()` drains both values, including after
`close()`. A Rust embedding application can also collect the sanitized tracing
events. Configure a collector and drain frequency appropriate to the workload.
The native package does not configure network telemetry export.

## Build and verify

From the repository root with the pinned Rust 1.94.0 toolchain and a standard
CPython environment:

```sh
python -m pip install 'maturin==1.16.0'
(cd python && maturin build --locked --release --out dist)
python -m pip install --no-deps python/dist/*.whl
python -m unittest discover -s python/tests -v
```

Run the tests from the repository root, not from `python/`, so the source
package does not shadow the installed extension. The native wheel tests perform
real local I/O and cover binary bytes, conditional collisions, policy/scope
denial, required confirmations, bounded ranges, cursor binding, symlinks,
case/Unicode aliases, file-only deletion, credential redaction and bounded audit.
The symlink case is skipped only when Windows denies fixture-creation privilege.
The fork case is skipped on platforms without `fork`.

Qualify S3 separately against a disposable emulator:

```sh
python -m pip install 'moto[server]==5.2.6'
python python/qualify_s3.py
```

The harness creates its own loopback server, bucket and synthetic credentials.
It verifies the emulator's conditional-create behavior before exercising native
S3 writes, ranges, collisions, signed GETs, listing, deletion and denied paths.
The server runs in the same Python process, so successful native HTTP operations
also exercise GIL release. It makes no calls to a paid provider and does not
qualify production deployment or provider-specific behavior.

The source-archive job separately checks strict Clippy, builds an sdist and
rebuilds its wheel outside the checkout before running the same tests. The
archive preparation removes workspace-only lock entries after Maturin selects
the Python crate and its path dependencies. It requires every retained package
version, source, checksum and retained dependency relationship to match the
qualified root lock, then proves locked offline resolution before deterministic
repacking. It also verifies
that the packaged MIT license exactly matches the repository license.

```sh
python scripts/test-python-sdist.py -v
python scripts/build-python-sdist.py --out python/dist
python -m pip wheel --no-deps --wheel-dir python/rebuilt \
    --config-settings=maturin.build-args=--locked python/dist/*.tar.gz
```

The archive-preparation script needs Python 3.11 or later, the pinned Rust
toolchain and the dependency cache populated by the preceding Rust checks.
The package itself retains its CPython 3.10 minimum. Rust
unit-test success alone is not the Python acceptance gate. The installed native
wheel and complete release matrix are required.
