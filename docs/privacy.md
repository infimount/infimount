# Privacy and Diagnostics

Infimount is local-first. Storage configuration, credentials, MCP settings, audit records, diagnostics, and product events are stored locally unless you explicitly export a diagnostics bundle or opt in to network telemetry.

## Product events and consent

Infimount always keeps a bounded local product-event log for diagnostics. Local events describe application flows such as activation steps and outcomes. The local log is separate from network consent: revoking consent stops future network export but does not delete local events. Use the separate Privacy control to clear them.

Network telemetry is off by default. It sends nothing unless both conditions are true:

1. the persisted Privacy preference is **granted**; and
2. an operator configures `OTEL_EXPORTER_OTLP_ENDPOINT` with HTTPS, or loopback HTTP for local testing.

The exporter sends schema-limited JSON events over HTTP through a bounded in-memory queue and a background worker with short connect/request timeouts. Tool calls never wait for network delivery. A full queue, unavailable endpoint, or failed response drops that best-effort event. Revoking consent prevents subsequent sends immediately. No Infimount-hosted telemetry service is required for the app to operate.

Eligible network events use fixed schemas. Product events contain an event name, schema version, timestamp, application version, operating-system/architecture category, and bounded non-sensitive properties. MCP operational metrics contain only an allowlisted tool name, allowlisted error code, or coarse duration bucket. They must not contain:

- storage credentials or MCP bearer tokens
- storage names, file contents, or prompts
- absolute local paths or object keys
- storage endpoints, bucket/container names, or config JSON
- presigned URLs or query signatures

The local product-event JSONL file and MCP audit log have independent size/rotation limits. The product-event log retains at most 5,000 events and 5 MiB, dropping the oldest records when either bound is exceeded.

## Diagnostics view

A diagnostics export performs a bounded same-version sidecar version/doctor check and reports its actual status. The summary also includes application version, platform category, native secret-store availability, configuration-file state, schema versions, storage/backend counts without names, enabled MCP tools, HTTP bind category, port state, and sanitized recent errors. Status is intended for troubleshooting, not as proof that a remote storage backend is healthy.

## Exported diagnostics bundle

The export contains:

- the diagnostics summary and sanitized error counts;
- up to 100 recent schema-limited product-event summaries;
- up to 100 recent MCP audit summaries containing only tool, operation, decision, coarse duration, and safe error code;
- a redaction manifest and SHA-256 checksums.

It excludes storage names, file paths and versions, bucket/container names, storage endpoints, config JSON, credentials, tokens, auth headers, file contents, prompts, session/confirmation identifiers, and presigned URL query strings. Before returning a successful export, Infimount validates both the in-memory bundle and every generated file against a corpus derived from sensitive local storage metadata/config values. Export directories and files use private local permissions where the platform supports Unix permission modes.

Review every exported bundle before sharing it. Redaction and corpus validation are defense in depth, not permission to publish a bundle indiscriminately.

## Deletion

Use Privacy settings to clear local product events. Revoking telemetry consent does not delete that separate local log. MCP audit data is managed separately in MCP Settings. Removing the app may not remove operating-system secret-store entries or user-created exports; delete those with the operating system's credential manager and file tools when required.
