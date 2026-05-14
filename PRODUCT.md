# Product

## Register

product

## Users

Infimount is for people who work across more than one storage system and need a single, calm place to browse, preview, move, and expose files safely.

Primary users:

- Desktop users who move between local folders, S3, Azure Blob Storage, Google Cloud Storage, and WebDAV.
- Developers and technical operators who want one storage browser instead of a separate tool for every backend.
- AI workflow users who expose selected storage sources to MCP clients such as local coding agents, LM Studio, or editor integrations.
- Operators who may run the MCP server in a local or controlled environment and need clear auth, scoping, and observability behavior.

The user is usually in a task, not exploring a marketing site. They are scanning file lists, checking file details, moving data, validating storage config, or deciding exactly what an MCP client can access.

## Product Purpose

Infimount exists to make storage feel like one local file-browser experience, even when the backend is local disk, object storage, or a remote WebDAV source.

The product should help users:

- Add and validate storage backends without memorizing backend-specific tooling.
- Browse files in grid and list views with predictable navigation.
- Preview files and inspect metadata without leaving the app.
- Move and copy files between supported storages where possible.
- Expose a controlled virtual filesystem to MCP clients.
- Keep storage registry, credentials, and MCP settings local by default.

Success means the app feels trustworthy, native, and quiet. A user should understand what is exposed, what is read-only, what is local, and what action is currently running without needing to study the implementation.

## Brand Personality

Minimal, native, careful.

Infimount should feel like a serious desktop utility, closer to the Ubuntu file manager than to a SaaS dashboard. The tone is direct and practical. The product does not need to look flashy to feel premium. It earns trust through clarity, restraint, and consistent behavior.

The brand should communicate:

- Local-first control.
- Storage unification without lock-in.
- Safe agent access through explicit MCP controls.
- Cross-platform utility that still feels native on each OS.

## Anti-references

Infimount should not look like:

- A glossy SaaS landing page inside the desktop app.
- A purple or blue gradient AI tool.
- A card-heavy marketing dashboard.
- A highly saturated icon playground.
- A dark cyber terminal unless the user explicitly chooses dark mode.
- A settings product that hides dangerous actions behind vague labels.
- A cloud console clone with dense, intimidating enterprise chrome.

Avoid decorative complexity. Avoid invented controls when a standard desktop pattern works better. Avoid color as decoration. Avoid animation that does not explain state.

## Design Principles

1. **Native first.** The desktop app should feel like a native file explorer: stable side navigation, clear file lists, quiet toolbar controls, and familiar modal behavior.
2. **Local-first is visible.** Storage config and credentials staying local is a product promise. Security-sensitive surfaces should say what is local, what is exposed, and what requires restart or confirmation.
3. **Restraint builds trust.** Most screens should use neutral greys, white surfaces, subtle borders, and small accent moments. Orange is an accent, not a theme.
4. **Show state before style.** Loading, selected, focused, read-only, running, stopped, exposed, disabled, and error states must be visible and consistent.
5. **Agent access is explicit.** MCP tools and storages should never feel magically exposed. The UI should make exposure, auth, bind address, and tool-level permissions obvious.

## Accessibility & Inclusion

Target WCAG AA for the desktop app and landing page.

Requirements:

- Keyboard navigation must be visible and usable across sidebars, dialogs, menus, and file lists.
- Focus states must not be removed without an equivalent visible replacement.
- Text contrast should meet WCAG AA for body text and controls.
- Destructive actions should use explicit confirmation dialogs with clear labels.
- Icon-only buttons require accessible labels or titles.
- Motion should be functional and subtle. Respect reduced-motion settings where animation becomes more than a quick state transition.
- Color must not be the only indicator of status. Pair status color with labels, icons, or structure.
