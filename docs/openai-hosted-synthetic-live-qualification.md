# Infimount v0.8.2 C2.6: opt-in hosted synthetic qualification

Status: **experimental operator-only compatibility pilot**, not a production
remote sandbox adapter, an Infimount task exporter or a source-storage write
facility. Stable v0.8.1, ordinary browsing and headless publication remain
unchanged. The CLI never reads local files or receives storage credentials.

## Offline mode

From the repository root, with Node.js 24:

```bash
node scripts/qualify-openai-hosted-synthetic.mjs
node --test scripts/qualify-openai-hosted-synthetic.test.mjs
```

Offline preview performs no API calls and needs no account. The automated
mock suite is also included in the normal CI job. CI must **never** set
`OPENAI_API_KEY` or use `--execute`.

## Optional actual provider qualification

The managed **Agents API beta** currently retains session state in the
**United States only** and **does not support Zero Data Retention**, even with
network egress disabled. The sandbox environment's
`network.access: "disabled"` controls sandbox egress, not provider data
residency or the provider's retention policy. The fixed prompt and output
are public synthetic text only. Do not upload user files or private data.

An operator who independently controls an OpenAI Platform project can
configure a restricted project API key with `api.agents.read`,
`api.agents.write` and `api.responses.write`, then explicitly authorize
**one** synthetic call:

```bash
node scripts/qualify-openai-hosted-synthetic.mjs \
  --execute \
  --accept-us-data-residency-and-non-zdr \
  --approve-one-usd-spend
```

The CLI reads `OPENAI_API_KEY` **only on this execution path**. It submits
one fixed, source-free session with `gpt-6-astra`, a `small` container,
`network.access: "disabled"`, no mounts/MCP/ambient credentials, and a
provider-side `spend_control.limit: 100` (USD cents). Provider spend
controls are **best effort**, and an account may still incur charges.
Use independent project billing limits. There is no automatic retry of a
possibly successful session creation.

The CLI streams events with byte and event budgets, captures the created
session ID as soon as it arrives, requires root-turn completion, fetches the
saved items with bounded opaque pagination, checks completed shell-command
exit codes, verifies the artifact's session/environment/turn/path and checks
its exact downloaded bytes against a fixed reference. It requests session
deletion even after subsequent failure, and refuses to report qualification
until deletion is confirmed.

**Interruption limitation:** If the provider creates a session but the
connection fails before the session ID is observed, automatic deletion
cannot be guaranteed. Inspect the project’s Agents logs before repeating
any paid run. If cleanup fails after an ID is observed, the CLI reports
`manual session cleanup required` with that opaque ID. Deletion is a public
API confirmation, not a guarantee of immediate physical erasure.

## Gates not satisfied by this pilot

- It never transfers prepared Infimount Agent Task inputs, fetches arbitrary
  user outputs, writes an Agent Workspace, or publishes original storage.
- The production flow still needs trusted native secret storage, explicit
  per-export byte-level consent, realistic spending consent, resilient
  session/turn recovery, sandbox output confinement, and persistent private
  execution evidence.
- The C1.3b in-memory importer is not a qualified OS no-follow, workspace-
  locked real-filesystem importer. Native human publication approval issues
  #181, #185 and #198 remain blocking.
- Success is an **API interoperability observation**, not general production
  readiness, sovereign deployment, compliance assurance or a guarantee every
  arbitrary agent task succeeds.

## Primary API references

- [Quickstart](https://developers.openai.com/api/docs/guides/agents-api/quickstart)
- [Sessions and recovery](https://developers.openai.com/api/docs/guides/agents-api/sessions)
- [Hosted environment](https://developers.openai.com/api/docs/guides/agents-api/environments/openai-hosted)
- [Artifacts](https://developers.openai.com/api/docs/guides/agents-api/environments/files)
- [Agents API residency / retention FAQ](https://help.openai.com/en/articles/20001551-agents-api-beta-faq)
