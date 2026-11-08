# Infimount v0.8.2 C2.4: offline hosted artifact inventory and byte gate

Status: **offline qualification only**. No OpenAI HTTP calls, credentials, paid
compute, host/cloud writes or final publication.

OpenAI's beta [Artifacts API](https://developers.openai.com/api/reference/resources/beta/subresources/agents/subresources/sessions/subresources/artifacts/methods/list)
lists immutable files from a completed hosted turn under
`/workspace/outputs`. Unlike C1's candidate listing, the provider artifact
metadata supplies a size, IDs and path **but no SHA-256**.

## Exact bounded flow

1. Rebuild the separately reviewed export bundle from trusted local task
   manifest and export brief. Require explicit file deliverables.
2. Receive authenticated, session-scoped artifact pages with `order=asc`,
   matching the exact session, environment and root turn of **one fresh task
   session**. Reject foreign/old artifacts, duplicates, cursor inconsistency,
   unsupported object shapes, unsafe paths, unexpected/missing outputs and
   size drift. Maximum 100 artifacts, 100 pages, 2 MiB JSON/page, 200 MiB
   per file, 500 MiB aggregate, matching the hosted API's output caps.
3. Accept completion only if an independently authenticated root turn ended
   successfully, with separate C2.3 saved-item outcome verification.
4. Download every immutable artifact from its exact listed ID. The trusted
   transport must bind the session and artifact ID, never an agent-authored
   URL. `validate_downloaded_hosted_artifacts` requires a 1:1 ID match,
   verifies the downloaded byte lengths and **computes local SHA-256**.
5. Revalidate the resulting provider-neutral C1.2 listing, stage the exact
   downloaded bytes, then use C1.3b reviewed-scope import which independently
   hashes the **staged bytes again** before its create-only in-memory write.
6. Final original-storage publication continues to require independent
   review and trusted human approval. These offline structs are **not**
   evidence of authorization.

## Limits and remaining production gates

The artifact inventory checks metadata before download and has no I/O.
For a future real importer, OS-level no-follow confinement, atomic locks,
private execution evidence, retries, cleanup and exact session/turn binding
remain required. Closing a provider stream is not cancellation.

A live OpenAI-hosted pilot requires user-owned API credentials, separate
per-execution export/spend consent and disclosure that the Agents API
currently offers **U.S. data residency only, without Zero Data Retention**.
Disabling sandbox networking is not equivalent to data residency or zero
retention. Neither this implementation nor CI uploads any data.

Sources:
- https://developers.openai.com/api/docs/guides/agents-api/environments/files
- https://developers.openai.com/api/docs/guides/agents-api/environments/openai-hosted
- https://help.openai.com/en/articles/20001551-agents-api-beta-faq

Stable v0.8.1 remains unchanged. The provider remains a replaceable compute
adapter and must not gain MCP source credentials or publication authority.
