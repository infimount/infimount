# Infimount Product and Market Study

Status: working product/market brief for agents and contributors  
Last reviewed: 2026-05-24

## 1. Product thesis

Infimount is a local-first desktop storage browser for people who work across multiple storage systems. It should make local folders, object storage, WebDAV, and future remote protocols feel like one predictable file manager, while letting users expose only selected storage access to AI agents through MCP.

The durable product wedge is not “another cloud file manager.” It is:

1. **Unified storage browsing** across local/cloud backends.
2. **Local-first credentials and config** with no Infimount-hosted backend required.
3. **Safe agent access** through MCP controls: exposed storages, enabled tools, path policies, confirmations, sessions, and audit logs.
4. **Native desktop utility UX** rather than a SaaS admin console.

## 2. Current repository baseline

### Desktop app

- Tauri + React desktop shell in `apps/desktop`.
- Main app surface: storage sidebar, file browser, preview panel/dialog, add/edit storage flows, MCP settings, first-run onboarding.
- UI stack: React 19, TypeScript, Vite, Tailwind, Radix UI, TanStack Query, Vitest, Playwright component testing.

### Rust core

- Shared core crate in `crates/core`.
- Uses Apache OpenDAL for storage operations.
- Desktop file operations are thin wrappers over OpenDAL: list, stat, read, write, create directory, delete, upload, transfer, versions.
- Registry currently builds OpenDAL operators for local filesystem, S3, Backblaze B2, Aliyun OSS, Tencent COS, Huawei OBS, Azure Blob, GCS, and WebDAV.

### MCP server

- MCP crate in `crates/mcp`.
- Supports stdio and Streamable HTTP.
- Storage registry persists locally at `~/.infimount/storages.json`.
- MCP settings persist locally at `~/.infimount/mcp_settings.json`.
- Filesystem tools route absolute virtual paths like `/StorageName/path.txt`.
- Storage-management tools support list/add/edit/remove/import/export/validate.
- Safe MCP controls include exposed/enabled flags, read-only, path policy, confirmation queue, audit log, and sessions.

### Documentation status

`Agents.md` now reflects the wired OpenDAL backends: local filesystem, S3/S3-compatible, native Backblaze B2, Azure Blob, Google Cloud Storage, and WebDAV. Future backend work should keep this guide aligned with the registry and desktop schema.

## 3. Primary user segments

### Segment A: developer/operator storage worker

Needs:

- Browse and move files across local folders, buckets, B2/OSS/COS/OBS buckets, containers, and WebDAV.
- Validate storage credentials quickly.
- Avoid switching among cloud consoles, CLIs, Cyberduck-style apps, and filesystem windows.

Buying/use trigger:

- “I touch several backends and need one calm browser.”

### Segment B: AI workflow user

Needs:

- Give an AI client access to selected files without granting the whole machine or all cloud credentials.
- See what the agent can access.
- Approve risky writes/deletes/download links.
- Audit what happened.

Buying/use trigger:

- “I want my local coding/AI agent to read or modify storage safely.”

### Segment C: privacy/security-conscious desktop user

Needs:

- Local-first credentials.
- Clear state for what is exposed or not exposed.
- No mandatory account or hosted control plane.

Buying/use trigger:

- “I want cloud convenience without a vendor backend holding my registry.”

## 4. Competitive landscape

This study uses general market knowledge available to the agent, not live web browsing.

### File transfer and cloud storage browsers

Representative competitors:

- Cyberduck / Mountain Duck
- ExpanDrive
- CloudMounter
- odrive
- Rclone Browser and rclone-based tools
- Transmit, ForkLift, Commander One
- Native cloud consoles from AWS, Azure, and GCP

Common strengths:

- Mature transfer workflows.
- Broad protocol support in some tools.
- Familiar file-manager metaphors.
- Some support mounted-drive workflows.

Common weaknesses/opportunities:

- AI-agent access is generally not the primary design center.
- Security controls are often credential/account oriented, not tool/path/confirmation oriented for agents.
- Cross-cloud UX can feel like a transfer client rather than a safe programmable filesystem.
- Some tools are proprietary, account-based, or platform-specific.

### Developer storage abstraction

Representative alternatives:

- rclone CLI
- s5cmd / awscli / azcopy / gsutil
- Apache OpenDAL ecosystem
- SDK-specific tools and custom scripts

Common strengths:

- Powerful automation and scripting.
- Mature backend coverage.
- Good for technical users.

Common weaknesses/opportunities:

- Not calm or visual for browsing/previewing.
- No native MCP safety UX out of the box.
- Less approachable for mixed desktop + AI workflows.

### MCP and agent filesystem tools

Representative alternatives:

- Generic filesystem MCP servers.
- Cloud-provider MCP servers.
- IDE/coding-agent file access tools.

Common strengths:

- Direct agent integration.
- Simple local file access.

Common weaknesses/opportunities:

- Often single-backend or broad local access.
- Weak human-in-the-loop approval UX.
- Limited storage registry and backend validation.
- Limited auditability and per-storage path policy.

## 5. Differentiation

Infimount can win by owning this position:

> The local-first storage browser built for safe human-controlled AI access.

Differentiators to preserve:

1. **Local-first trust**: registry, credentials, settings, audit stay on the user machine by default.
2. **MCP-native safety**: tool enablement, storage exposure, path policy, confirmations, sessions, and audit are product primitives.
3. **Unified OpenDAL backend**: storage breadth without reimplementing backend semantics.
4. **Native utility design**: practical file-manager UX, low visual noise, explicit dangerous-action copy.
5. **Cross-platform distribution**: Linux/macOS/Windows binaries plus Homebrew.

## 6. Product risks

1. **Backend breadth vs reliability**: supporting many backends can create edge cases. Mitigation: use OpenDAL, validate capabilities, document backend differences.
2. **MCP security complexity**: powerful tools can become dangerous. Mitigation: secure defaults, explicit exposure, read-only controls, confirmations, audit.
3. **UX density in MCP settings**: safety controls can become overwhelming. Mitigation: “What the agent can access” summary and progressive disclosure.
4. **Docs drift**: product claims, agent docs, README, and implementation can diverge. Mitigation: doc consistency checks as part of agent workflow.
5. **Trust loss from secret leakage**: logs, exports, UI, or tests must not reveal credentials. Mitigation: masking rules and secret grep before release.

## 7. Roadmap opportunities

### Near-term product quality

- Keep docs aligned with the current backend set, MCP safety model, and release status.
- Continue keyboard navigation hardening in the sidebar, dialogs, and MCP settings; file grid/table roving navigation is implemented.
- Improve large directory performance: virtualized list/grid is implemented, and global search indexing can ignore stale cancelled responses; continue pagination/cursor where possible and async progress states.
- Strengthen storage validation with clearer capability summaries; grouped validation results, sanitized fix hints, MCP readiness notes, and copyable summaries are now started for v0.7.0.
- Improve error messaging for backend-specific auth/config failures.

### MCP and AI workflow

- Keep the “What the agent can access” summary the central MCP settings primitive.
- Expand scenario presets beyond the implemented read-only research, workspace agent, manual-approval, and lockdown modes.
- Continue hardening HTTP auth and non-loopback warnings; desktop non-loopback HTTP now requires a bearer token.
- Continue improving audit filtering/export without leaking secrets; text/decision/storage filters, copy-visible JSON, and redacted export-visible bundles are implemented.
- Expand MCP scenario tests beyond the implemented allowed-read, denied-prefix, confirmation, read-only-session, cross-storage-copy, and audit-redaction cases.

### Storage breadth

- SFTP and FTP backends.
- More S3-compatible provider presets. Cloudflare R2, MinIO, Wasabi, Backblaze B2 S3-compatible endpoints, DigitalOcean Spaces, and Nextcloud/WebDAV presets are implemented in the add-storage flow; native Backblaze B2, Aliyun OSS, Tencent COS, and Huawei OBS are also available as first-class OpenDAL backends.
- Better WebDAV/Nextcloud guidance.
- Versioning UI where backend supports it.

### Power-user workflows

- Multi-tab browsing.
- Transfer queue and resumable/retry behavior where possible. Queued/running/completed/failed UI, retry, active or queued cancellation, Tauri-backed progress events, and persisted transfer history are implemented. True resume remains future work only if it can be implemented through OpenDAL-exposed capabilities without provider-specific storage code.
- Dual-pane browsing for source/destination work. Initial split-pane browsing with an independently selectable destination pane is implemented.
- CLI companion.
- Saved connections/searches.

## 8. Design guardrails

- Keep product surfaces file-manager-like, not SaaS-dashboard-like.
- Orange is rare accent only.
- Every security-sensitive switch needs an effect description.
- Dangerous actions need explicit confirmation copy.
- Icon-only controls require accessible labels.
- Focus states must remain visible.
- State before style: selected, focused, loading, running, stopped, exposed, read-only, denied, pending approval, and error must be obvious.

## 9. Agent operating implications

Any agent working on Infimount should act as:

1. **Product owner**: preserve local-first trust and explicit safety.
2. **Developer**: keep storage logic in Rust core/MCP OpenDAL layers; keep Tauri and React thin. Do not add provider-specific SDK paths for file operations outside the OpenDAL abstraction.
3. **Tester**: add regression tests for each behavior and verify secret masking, policy denial, and accessibility basics.
4. **Documentarian**: update README/docs/Agents when product behavior changes.

## 10. Quality gates

Before merging meaningful changes, run the relevant subset:

```bash
cargo test --workspace
cd apps/desktop
pnpm test:unit
pnpm test:integration
pnpm lint
pnpm build
```

For release or security-sensitive changes, also run:

```bash
git grep -nE "(AKIA|BEGIN PRIVATE KEY|AIza|SECRET|TOKEN)"
bash scripts/check-release-links.sh
```
