# Product

## Register

product

## Product thesis

Infimount is **one surface for every storage**: a local-first, cross-platform
file browser and storage workbench for local files, object stores, cloud drives,
and remote file services.

Its primary job is to make everyday file operations predictable across
heterogeneous backends: connect, browse, preview, search, upload, download,
organize, compare, and transfer. The desktop file browser is the product's
primary experience and must remain useful without enabling AI features.

**Agent Access and Agent Tasks are optional extensions.** For users who choose
them, Infimount provides a governed data plane: explicit source selection,
least-privilege MCP access, isolated tasks, review and create-only publication.
Agents consume the storage workbench's capabilities. Storage browsing does not
depend on any agent, MCP connection, headless service or hosted backend.

## Primary users

- People who manage files across local disks, buckets, cloud drives, WebDAV,
  and SFTP, including non-developers who never enable Agent Access.
- Developers and technical operators transferring, inspecting and comparing
  source artifacts across storage systems.
- Teams and operators needing backend-neutral file operations with clear
  capabilities, progress, conflict choices and reliable recovery.
- Developers using AI agents who voluntarily opt into scoped data ingress,
  execution evidence and reviewed write-back.

FTP records may remain from older versions, but FTP operations remain disabled
in v0.8 while the documented upstream security issue is unresolved.

## Product purpose

**Default storage workflow:**

```text
connect + validate storage
         |
         v
browse / preview / search
         |
         v
upload / download / organize / copy / move / compare
         |
         v
visible progress + conflict resolution + recovery
```

**Optional agent workflow:**

```text
explicitly selected source files
         |
         v
bounded snapshot + provenance
         |
         v
scoped agent execution
         |
         v
output review + explicit approval
         |
         v
verified create-only publication + receipt
```

The file browser must preserve grid and list views, folder navigation and
pagination, previews, bookmarks and recents, keyboard access, opt-in search,
drag-and-drop upload, downloads, create/rename/delete, cross-storage transfers,
dual-pane workflows, visible progress, retry/cancel and safe conflict handling.
Backend differences must be capability-gated instead of hidden.

Success for a **storage-only** user means installing the app, adding storage,
browsing and completing ordinary file operations without configuring MCP.
Success for an **agent** user additionally means answering what data was
shared, which tools were allowed, which outputs were reviewed, and where
approved bytes will be published.

## Strategic boundaries

Infimount should **not** become:

- a built-in LLM or agent runtime;
- a generic MCP gateway, proxy, or marketplace;
- a semantic/vector/RAG ingestion platform;
- a replacement for provider-native MCP servers;
- a provider-specific storage SDK layer outside the OpenDAL-first architecture;
- a cloud collaboration suite before the local/headless policy contract is mature.

Those systems are integration targets. Infimount's core value is a dependable cross-storage file browser; optional agent workflows add governed ingress and egress without replacing that browser.

## Product scope and decision gate

Brainstorming, competitor research and a feature request are **not approval
to implement**. A new major product track (for example distributed shared
workspaces, hosted services, sync, collaborative editing or a new runtime)
requires an explicit recorded decision before engineering starts:

1. Identify the user problem and why existing browser/Git/provider tooling is
   insufficient.
2. Compare alternatives, dependencies, maintenance cost and security impact.
3. Show how the original file-browser experience stays first-class, including
   any UI complexity added to storage-only users.
4. Define a bounded scope, acceptance criteria and automated regression gates.
5. Record approval in a product decision/ADR or linked issue. Only then schedule
   implementation PRs.

Exploratory ideas can be documented but must remain uncommitted to the roadmap
until approved. Refactors in an already-approved capability track must still
preserve browser behavior.

## v0.8.2 direction

v0.8.2 is the active feature train. Its **non-negotiable release baseline**
is the storage workbench: ordinary browsing and mutations, storage-only
onboarding, transfers, previews and supported backend behavior continue to
work before new agent functionality can be released.

Approved optional agent directions remain:

1. **Agent-neutral Agent Tasks.** Codex remains supported behind a stable handoff contract.
2. **Additional local adapters.** Claude Code, Gemini CLI, OpenCode/Pi only after qualification.
3. **Trusted headless control.** Reuse desktop authorization, locks and review rules for CLI/API.
4. **Interactive agent-host review.** MCP Apps-compatible surfaces where safely supported.
5. **Sandbox interoperability.** Use outside runtimes rather than building one.
6. **Portable trust evidence.** Preserve identity, policy, provenance, receipt and audit semantics.

Browser regressions block release. Experimental agent work must never silently
enable agent access or require a workspace to use the file browser.

No v0.9 release is planned for roughly the next year.

## Brand personality

Minimal, native, careful.

Infimount should feel like a serious desktop utility and trustworthy infrastructure boundary, not a glossy AI dashboard. The tone is direct and practical. The product earns trust through clarity, restraint, inspectable state, and predictable behavior.

The brand should communicate:

- Local-first control.
- One surface across heterogeneous storage.
- Familiar cross-storage file workflows.
- Optional, explicit agent data boundaries.
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

1. **Browser first.** Browsing and ordinary file operations must work with Agent Access disabled; product and release quality begin there.
2. **Boundaries before intelligence.** Define what data and authority an agent receives before optimizing agent capabilities.
3. **Native first.** The desktop app should remain a calm, familiar storage workbench.
4. **Local-first is visible.** State what remains local, what is exposed, and what crosses a process or network boundary.
5. **Approve boundaries, not chatter.** Prefer scoped autonomy inside a prepared task and one explicit publication decision over repeated low-level permission prompts.
6. **Show state before style.** Loading, selected, focused, read-only, running, stopped, exposed, disabled, reviewed, approved, stale, and failed states must be visible.
7. **Agent access is explicit.** MCP exposure, transport, storage scope, path policy, and tool authority must never be implicit.
8. **Write-back is a separate trust boundary.** Agent completion never implies permission to publish.

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
