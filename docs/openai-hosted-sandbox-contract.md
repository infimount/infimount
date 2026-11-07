# Infimount v0.8.2 C2.1: OpenAI-hosted sandbox offline request

Status: **pure request constructor with no HTTP/session creation, uploads,
credential retrieval or paid model invocation**.

## Why this is a separate adapter

OpenAI has two independently documented execution models:

- [Agents SDK SandboxAgent](https://developers.openai.com/api/docs/guides/agents/sandboxes)
  leaves the agent harness inside the application and selects a sandbox client.
- [Agents API OpenAI-hosted sandbox](https://developers.openai.com/api/docs/guides/agents-api/environments/openai-hosted)
  runs an OpenAI-managed harness, session and Linux workspace.

C2.1 targets only the second model and must not redefine Infimount's
provider-neutral ingress/egress contracts. No provider is trusted to select
local storage destinations or authorize final publication.

## Offline construction and safety

`openai_hosted_request::build_openai_hosted_offline_request` accepts:

- A validated, locally prepared Agent Task manifest.
- A **separately reviewed and redacted** export brief.
- Exact task-relative input byte slices, with file name, size, and SHA-256
  revalidated against the manifest at request construction time.
- A bounded model identifier chosen by a future trusted user-facing flow.

The returned payload is a tightly allowlisted v1 Agents API session request:

- `agent.model` and a fixed guidance prompt.
- `environment.type: "openai_hosted"`.
- `environment.network.access: "disabled"`, **always explicit**.
- `environment.files[]`: inline base64 bytes for only reviewed `inputs/`
  paths inside `/workspace/`. No host mounts or source storage credentials.
- The reviewed export objective plus a **canonical sorted list** of every
  requested `outputs/` deliverable as `input`, and `stream: true`.
  These paths are instructions to the agent, not permission to write local
  storage or publish files. Future output import still independently
  inventories and hashes provider artifacts against the user's review.

No `env`, `setup_commands`, `packages`, `mounts`, external `mcp`,
source storage ID or API token field is included. The request struct does
not derive `Debug` because it contains reviewed user data.

The first inline-only pilot caps inputs to **32 files / 8 MiB** and the
complete serialized request to **12 MiB**. Larger inputs require a separately
approved File-ID upload stage and independently verified byte evidence.
The OpenAI documentation's hosted-sandbox networking default is enabled,
so **never omit** the explicit disabled access setting.

A model prompt is not a security boundary. Only provider-enforced egress
controls and separate Infimount authorization can protect real resources.

## What is deliberately not yet implemented

This module makes **no** API calls, does not store credentials, does not
install an embedded Node/Python harness and cannot spend money. A future C2
service must qualify token management in native secret storage, session
create/turn streaming, resource/network policy, bounded artifact download
and independent C1.2/C1.3 byte verification before a create-only import.

The OpenAI-hosted sandbox publishes copies of files beneath
`/workspace/outputs/` as artifacts. A session turn completing does **not**
guarantee every tool succeeded. Closing an event stream does **not** cancel
the task, and session deletion needs bounded retries. See
[hosted sandbox lifecycle](https://developers.openai.com/api/docs/guides/agents-api/environments/openai-hosted)
and [Agents API quickstart](https://developers.openai.com/api/docs/guides/agents-api/quickstart).

Future C2 live tests require explicit provider configuration, user-reviewed
source export, consumption and spending limits, and a synthetic coding,
document and data-analysis pilot. No normal local/S3/Azure/GCS/WebDAV browsing
should require this adapter.

Keep v0.8.1 stable, headless final publication disabled, and B1 native
approval issues #181/#185/#198 open. See [C2 issue](https://github.com/infimount/infimount/issues/218)
for the live qualification contract.
