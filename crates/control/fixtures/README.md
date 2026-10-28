# Native publication presence v1: canonical test vector

This directory contains one **synthetic, read-only protocol vector** for the future
Infimount v0.8.2 native user-verification adapters. It is not an issued grant or
evidence of a user's presence.

- File: `native-publication-presence-v1.canonical.json`
- Exact UTF-8 byte count: **788**
- SHA-256 of those bytes (lowercase hex):
  `65c77d7313bea4f4fa9242c779bf64a33718aadfc87a83bec6fe2e3044595aff`
- The file has **no BOM, spaces, or trailing newline**. Field ordering and
  serialization are significant. Do not parse and pretty-print before hashing.
- The actor, UUIDv4 session and challenge nonce, task, reviewed source hash,
  destination, revision, timestamps and path decisions are synthetic test data.

This vector is pinned in `publication_presence_challenge.rs`. A platform
adapter must prove that its verifier binds the same challenge **digest bytes**
to a valid credential's signed assertion using that platform's **actual**
signature protocol. WebAuthn client data, authenticator data, UP/UV flags, and
RP/credential binding are not interchangeable with a direct signature on
32 arbitrary bytes. macOS signature algorithms likewise must not double-hash
the digest accidentally.

No test fixture may be treated as a production credential, issuer, grant,
hardware user-verification evidence, or headless publication authority.
See Proposed ADR 0011 and OS qualification issue #198. Unsupported authenticators
must leave publication disabled while preserving normal desktop browsing.
