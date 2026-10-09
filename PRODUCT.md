# Product

## Register

product

## Product thesis

Infimount is a **local-first governed data plane for AI agents**.

It connects heterogeneous systems of record to agent environments without making the agent environment itself the source of authority. Infimount decides which data may enter a task boundary, preserves source provenance, keeps agent work scoped, and controls exactly which reviewed bytes may be written back.

The desktop storage browser remains a first-class human control surface. It is no longer the whole product thesis.

## Primary users

- Developers and technical operators running coding or knowledge agents against files distributed across local storage, object stores, cloud drives, WebDAV, and SFTP.
- Data owners who need agents to work on a bounded subset of real files without granting broad direct access to the source system.
- Desktop users who still need one calm place to browse, preview, validate, move, and manage files across heterogeneous storage.
- Security-conscious operators who need explicit MCP exposure, path policy, read-only controls, approvals, audit, provenance, and local credential handling.
- Teams integrating external agent runtimes or sandboxes that need a provider-neutral ingress/egress layer for storage data.

FTP records may remain from older versions, but FTP operations remain disabled in v0.8 while the documented upstream security issue is unresolved.

## Product purpose

Infimount should make this workflow safe and understandable:

```text
systems of record
      ↓
explicit selection
      ↓
snapshot + hashes + provenance
      ↓
bounded agent workspace
      ↓
agent/runtime/sandbox
      ↓
reviewed outputs
      ↓
exact approved publication plan
      ↓
verified create-only write-back + receipt
      ↓
systems of record
```

The product should help users:

- Add and validate storage backends without memorizing provider-specific tooling.
- Browse and inspect files with predictable desktop workflows.
- Define explicit agent-access boundaries instead of implicitly exposing whole storage accounts.
- Prepare bounded task data without mutating source bytes or broadening source MCP exposure.
- Let external agents work autonomously inside an approved boundary rather than prompting on every low-level file operation.
- Review outputs and provenance before anything leaves the task workspace.
- Publish only explicitly approved bytes to explicitly approved destinations.
- Preserve local-first configuration, native secret storage, auditability, and backend-agnostic OpenDAL I/O.

Success means a user can answer four questions at any moment:

1. What source data did the agent receive?
2. What authority did the agent have while working?
3. What output bytes are being proposed?
4. What exactly will be written back, where, and under whose approval?

## Strategic boundaries

Infimount should **not** become:

- a built-in LLM or agent runtime;
- a generic MCP gateway, proxy, or marketplace;
- a semantic/vector/RAG ingestion platform;
- a replacement for provider-native MCP servers;
- a provider-specific storage SDK layer outside the OpenDAL-first architecture;
- a cloud collaboration suite before the local/headless policy contract is mature.

Those systems are integration targets. Infimount's differentiated responsibility is governed data ingress and egress across heterogeneous storage.

## v0.8.2 direction

v0.8.2 is the active feature train.

Primary directions:

1. **Agent-neutral Agent Tasks.** Codex remains supported but becomes one adapter behind a stable task/handoff contract.
2. **Additional agent adapters.** Prioritize Claude Code, Gemini CLI, OpenCode/Pi, and compatible MCP clients based on real integration quality.
3. **Headless control surface.** Add CLI/API access for task preparation, inspection, review, publication, and evidence without bypassing the same policies used by the desktop.
4. **Interactive review in agent hosts.** Use MCP Apps-compatible UI where available for output review and publication approval.
5. **Sandbox interoperability.** Integrate isolated agent runtimes rather than building an Infimount runtime; Infimount owns the bounded data room and safe write-back contract.
6. **Portable trust metadata.** Strengthen agent identity, policy, provenance, receipts, audit, and observability so task evidence survives across clients and runtimes.

No v0.9 release is planned for roughly the next year. The objective is to deepen and prove the v0.8 architecture, not to use version numbers as product milestones.

## Brand personality

Minimal, native, careful.

Infimount should feel like a serious desktop utility and trustworthy infrastructure boundary, not a glossy AI dashboard. The tone is direct and practical. The product earns trust through clarity, restraint, inspectable state, and predictable behavior.

The brand should communicate:

- Local-first control.
- One surface across heterogeneous storage.
- Explicit agent data boundaries.
- Review-before-write-back.
- Backend and agent neutrality.
- Cross-platform utility without infrastructure lock-in.

## Anti-references

Infimount should not look or behave like:

- A card-heavy SaaS marketing dashboard inside the desktop app.
- A generic purple/blue-gradient AI wrapper.
- A chat application pretending to be a storage control plane.
- A settings product that hides dangerous actions behind vague labels.
- A cloud console clone with dense enterprise chrome.
- An agent runtime that silently acquires broader filesystem authority.
- A workflow that asks for approval on every low-level operation instead of establishing a clear bounded workspace.

Avoid decorative complexity. Avoid invented controls when a standard desktop pattern works better. Avoid color as decoration. Avoid animation that does not explain state.

## Design principles

1. **Boundaries before intelligence.** Define what data and authority an agent receives before optimizing what the agent can do.
2. **Native first.** The desktop app should remain a calm, familiar storage control surface.
3. **Local-first is visible.** State what remains local, what is exposed, and what crosses a process or network boundary.
4. **Approve boundaries, not chatter.** Prefer scoped autonomy inside a prepared task and one explicit publication decision over repeated low-level permission prompts.
5. **Show state before style.** Loading, selected, focused, read-only, running, stopped, exposed, disabled, reviewed, approved, stale, and failed states must be visible.
6. **Agent access is explicit.** MCP exposure, transport, storage scope, path policy, and tool authority must never be implicit.
7. **Write-back is a separate trust boundary.** Agent completion never implies permission to publish.

## Accessibility & inclusion

Target WCAG AA for the desktop app and landing page.

Requirements:

- Keyboard navigation must be visible and usable across sidebars, dialogs, menus, file lists, and review/publication flows.
- Focus states must not be removed without an equivalent visible replacement.
- Text contrast should meet WCAG AA for body text and controls.
- Destructive actions should use explicit confirmation dialogs with clear labels.
- Icon-only buttons require accessible labels or titles.
- Motion should be functional and subtle and respect reduced-motion settings.
- Color must not be the only indicator of status.
