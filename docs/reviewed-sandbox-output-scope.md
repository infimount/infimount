# Infimount v0.8.2 C1.3b: exact reviewed output scope

The first remote-sandbox pilot must not treat a provider's self-declared
`outputs/` listing as authorization to import arbitrary files. C1.2 validates
listing paths and declared hashes. C1.3a checks actual staged bytes and
performs create-only **in-memory** transfers. Neither independently proves
the listing matches the separately reviewed export deliverables.

The new `sandbox_reviewed_ingest` module enforces that missing or unexpected
files fail closed **before writing any destination file**:

1. Rebuild the reviewed `SandboxTaskBundle` from the trusted prepared task
   manifest and separately approved export brief.
2. Independently revalidate the candidate listing and its metadata digest.
3. Require an **exact sorted file-path match** between requested deliverables
   and every candidate entry. No extra file, missing file, or path substitution.
4. Only then invoke the existing C1.3a in-memory importer, which rechecks
   entire staging inventory, rehashes actual bytes and uses create-only writes.

This prototype does not provide an OS-confined real filesystem importer.
A production path still requires no-follow file handles, locks, private
execution evidence, explicit export consent, separate output review,
native human approval and provider-specific synthetic end-to-end tests.
It grants no headless publication authority and never writes cloud storage.
