<p align="center">
  <img src="docs/assets/infimount-logo-text.png" alt="Infimount" width="520"/>
</p>

<p align="center">
  <strong>Local-first storage browser for desktop and MCP workflows.</strong><br/>
  Browse local folders, object storage, and WebDAV from one native app. Expose only selected storages to AI clients.
</p>

<p align="center">
  <a href="LICENSE"><img src="https://img.shields.io/badge/License-MIT-yellow.svg" alt="License: MIT"/></a>
  <a href="CODE_OF_CONDUCT.md"><img src="https://img.shields.io/badge/Contributor%20Covenant-2.1-4baaaa.svg" alt="Contributor Covenant"/></a>
  <a href="https://github.com/infimount/infimount/actions/workflows/ci.yml"><img src="https://github.com/infimount/infimount/actions/workflows/ci.yml/badge.svg" alt="CI"/></a>
  <a href="https://github.com/infimount/infimount/releases"><img src="https://img.shields.io/github/v/release/infimount/infimount?include_prereleases" alt="Release"/></a>
  <a href="https://github.com/sponsors/infimount"><img src="https://img.shields.io/github/sponsors/infimount?style=social" alt="GitHub Sponsors"/></a>
</p>

<p align="center">
  <img src="docs/assets/screenshot-infimount.png" alt="Infimount desktop app screenshot" width="900" />
</p>

> **Local-first by default**
>
> Infimount stores storage sources, app config, MCP settings, and credentials on your machine.
> Default storage registry: `~/.infimount/storages.json`.
> MCP runtime settings: `~/.infimount/mcp_settings.json`.
> No Infimount-hosted backend is required.

## Install

**Current stable release:** [v0.7.0](https://github.com/infimount/infimount/releases/tag/v0.7.0)

### Linux

```bash
curl -fsSL https://github.com/infimount/infimount/releases/latest/download/install.sh | sh
```

The script verifies checksums and chooses `.deb`, `.rpm`, or AppImage automatically. Override with `INFIMOUNT_INSTALL_FORMAT=deb|rpm|appimage`.

Manual downloads:

- [DEB for Debian/Ubuntu](https://github.com/infimount/infimount/releases/latest/download/Infimount-amd64.deb)
- [RPM for Fedora/RHEL](https://github.com/infimount/infimount/releases/latest/download/Infimount-x86_64.rpm)
- [AppImage for portable use](https://github.com/infimount/infimount/releases/latest/download/Infimount-x86_64.AppImage)

### macOS

```bash
curl -fsSL https://github.com/infimount/infimount/releases/latest/download/install.sh | sh
```

Or use Homebrew:

```bash
brew tap infimount/infimount
brew install --cask infimount
```

Manual download: [Infimount.dmg](https://github.com/infimount/infimount/releases/latest/download/Infimount.dmg)

### Windows

Run in PowerShell:

```powershell
irm https://github.com/infimount/infimount/releases/latest/download/install.ps1 | iex
```

Manual downloads:

- [MSI installer](https://github.com/infimount/infimount/releases/latest/download/Infimount.msi)
- [Setup EXE](https://github.com/infimount/infimount/releases/latest/download/Infimount-setup.exe)

### Install notes

Install scripts verify selected downloads against `SHA256SUMS.txt`. To pin a version, set `INFIMOUNT_VERSION=v0.7.0` before running the command. macOS and Windows binaries may be unsigned for some releases, see [Installation Notes](#installation-notes).

## What Infimount does

- **Browse storage in one place:** local files, S3/S3-compatible storage, Backblaze B2, Aliyun OSS, Tencent COS, Huawei OBS, Azure Blob, Google Cloud Storage, and WebDAV.
- **Work like a desktop file manager:** grid and list views, rich previews, drag-and-drop upload, bookmarks, recents, keyboard navigation, global search stop, dual-pane transfer workflows, conflict handling, and transfer queue.
- **Validate before you trust a backend:** reachability checks report grouped capabilities, sanitized fix hints, and MCP readiness notes.
- **Control MCP access explicitly:** new storages are not exposed to MCP by default. Enable selected storages, tool lists, path policies, read-only mode, confirmations, and local audit logs.
- **Stay backend-agnostic:** file operations route through Apache OpenDAL so capabilities are detected and documented per backend.

## Workbench

Infimount includes daily file-manager workflows beyond basic browsing:

- Dual-pane copy, move, compare, and update flows across supported storages.
- Transfer queue with queued/running/completed/failed states, retry, active or queued cancellation, progress visibility, and persisted transfer history.
- Conflict handling for overwrite, discard, or keep-both transfers.
- Bookmarks, recent folders, drag-and-drop upload, rich preview, and roving keyboard navigation in grid/table views.
- Opt-in global search indexing with a Stop control so stale slow-storage responses do not overwrite newer UI state.

## Agent Workspaces

Agent Workspaces give AI workflows a safer project-shaped storage area:

- Create coding, research, or data-analysis workspaces on OpenDAL-backed storage.
- Apply a workspace-scoped MCP policy that defaults to no access and allows only the workspace root.
- Keep visible memory files under `memory/` for task notes and handoff context.
- Capture checkpoint manifests under `.infimount/checkpoints` and restore workspace memory when needed.
- Review workspace activity grouped from local events and MCP audit events that fall under the workspace root.

## First run and upgrades

GitHub shows a copy button on each fenced command block in this README.

Linux AppImage:

```bash
chmod +x Infimount-*.AppImage
./Infimount-*.AppImage
```

Linux DEB:

```bash
sudo apt install ./Infimount-amd64.deb
```

Linux RPM:

```bash
sudo rpm -i Infimount-x86_64.rpm
```

macOS DMG: open the DMG, drag Infimount to Applications, then right-click and choose `Open` on first launch if Gatekeeper warns.

Windows MSI or EXE: run the installer. If SmartScreen appears, choose `More info` then `Run anyway`.

Upgrade by running the latest installer again. For Homebrew installs:

```bash
brew update
brew upgrade infimount
brew upgrade --cask infimount
```

## Build from source

See [Building from Source](#️-building-from-source) below.

---

## Supported Storage Backends

| Backend                         | Status     | Notes                                                                       |
| ------------------------------- | ---------- | --------------------------------------------------------------------------- |
| **Local Filesystem**            | ✅ Stable  | Full read/write support                                                     |
| **Amazon S3 / S3-compatible**   | ✅ Stable  | Any S3-compatible service; versioning depends on bucket support; optional default object ACL |
| **Backblaze B2**                | ✅ Stable  | Native OpenDAL B2 backend with read/write/list/delete, copy, presign, and capability-gated user metadata writes |
| **Aliyun OSS**                  | ✅ Stable  | Object storage via OpenDAL; read/write/list/delete/copy and presigned links; no generic rename/create-dir capability |
| **Tencent COS**                 | ✅ Stable  | Object storage via OpenDAL; read/write/list/delete/copy and presigned links; no generic rename/create-dir capability |
| **Huawei OBS**                  | ✅ Stable  | Object storage via OpenDAL; read/write/list/delete/copy and presigned links; no generic rename/create-dir capability |
| **Azure Blob Storage**          | ✅ Stable  | Container/account key auth; advanced capabilities depend on account support |
| **Google Cloud Storage**        | ✅ Stable  | Service account JSON; advanced capabilities depend on bucket support        |
| **WebDAV**                      | ✅ Stable  | Nextcloud, ownCloud, etc.; optional compatibility mode for servers that cannot create collection placeholders |
| **SFTP**                        | 🔜 Planned | Coming soon                                                                |
| **FTP**                         | 🔜 Planned | Coming soon                                                                |

Use **Validate** in Add/Edit Storage to check reachability, grouped capability summaries, sanitized fix hints, and MCP readiness notes before browsing or exposing a storage to agents.
For MCP/versioning details, see [Backend Capability Matrix](docs/backend-capabilities.md).

---

## 🤖 MCP Integration

Infimount includes a Rust MCP server for local AI clients and agent workflows.

- Transports: stdio and Streamable HTTP
- HTTP auth: bearer token required for non-loopback desktop HTTP and for headless HTTP unless explicitly started in loopback-only insecure dev mode
- Scoped access: new storages are not exposed to MCP by default; expose only selected storages, disable individual MCP tools, and restrict storage paths with allow/deny prefixes
- Risk controls: write/delete/presign/version-delete operations can require approval in Infimount before execution
- Audit trail: local bounded MCP audit log records allowed, denied, confirmed, and failed tool activity without storing secrets or presigned URL signatures
- Version-aware tools: supported where the backend and storage configuration support object versions

Setup guide: [MCP Client Setup](docs/mcp-client-setup.md)

Agent integration guide: [Agent Integrations](docs/agent-integrations.md)

Security model: [Security Model](docs/security.md)

---

## 🛠️ Building from Source

### Prerequisites

- **Rust 1.85+** (latest stable recommended) — [rustup.rs](https://rustup.rs/)
- **Node.js 18+** and **pnpm** — [pnpm.io](https://pnpm.io/installation)
- **Tauri dependencies** — [Platform-specific setup](https://tauri.app/start/prerequisites/)

### Quick Start

```bash
# Clone the repository
git clone https://github.com/infimount/infimount.git
cd infimount

# Install frontend dependencies
cd apps/desktop
pnpm install

# Run in development mode
pnpm tauri dev
```

### Build for Production

```bash
cd apps/desktop
pnpm build          # Build React frontend
pnpm tauri build    # Bundle native app
```

Outputs:

- **Linux**: `target/release/bundle/deb/`, `bundle/rpm/`, `bundle/appimage/`
- **macOS**: `target/release/bundle/dmg/`, `bundle/macos/`
- **Windows**: `target/release/bundle/msi/`, `bundle/nsis/`

> 📖 For release operations and checklist, see [docs/releasing.md](docs/releasing.md).
> To verify public download links before announcing a release, run `scripts/check-release-links.sh`.

---

## 🎯 Roadmap

### Current Focus

- [x] Local, S3/S3-compatible, Backblaze B2, Aliyun OSS, Tencent COS, Huawei OBS, Azure Blob, GCS, and WebDAV browsing
- [x] Grid and list views with file preview, drag-and-drop upload, bookmarks, recents, and transfer queue
- [x] Dual-pane copy/move and compare/update workflows
- [x] MCP support for local AI assistants with explicit storage exposure, tool controls, path policy, confirmations, sessions, and audit
- [x] Version-aware MCP tools where supported by the backend
- [x] Keyboard navigation in virtualized file grid and table views
- [ ] Additional storage backends such as SFTP and FTP
- [x] Capability-aware storage validation summaries with fix hints and MCP readiness notes
- [ ] Additional large-directory polish

### Future Plans

- [ ] CLI companion (`infimount-cli`)
- [ ] Mobile app (iOS/Android)
- [ ] Hosted and managed deployment options

---

## 🤝 Contributing

We welcome contributions! Please read:

- [CONTRIBUTING.md](CONTRIBUTING.md) — How to contribute
- [CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md) — Community standards
- [GOVERNANCE.md](GOVERNANCE.md) — Decision-making process
- [Agents.md](Agents.md) — Guidelines for AI assistants

### Development Commands

```bash
# Run tests
cd apps/desktop && pnpm test        # Frontend tests
cargo test --workspace               # Rust tests

# Lint & format
pnpm lint                            # ESLint
cargo fmt --check                    # Rust formatting
cargo clippy                         # Rust lints

# Enable local pre-commit checks (yamllint, markdownlint, actionlint)
pnpm setup:hooks
```

---

## 💖 Support the Project

If Infimount is useful to you, consider supporting its development:

<p align="center">
  <a href="https://github.com/sponsors/infimount">
    <img src="https://img.shields.io/badge/Sponsor-❤-ea4aaa?style=for-the-badge&logo=github-sponsors" alt="Sponsor on GitHub" />
  </a>
</p>

Your sponsorship helps:

- Maintain and improve the codebase
- Add new storage backends
- Keep Infimount free and open source

---

## 📝 Installation Notes

### macOS

Binaries may be unsigned/not notarized in some releases. To open:

1. Open the `.dmg`
2. Drag `Infimount.app` to `Applications`
3. Right-click the app and select `Open`
4. Click `Open` in the dialog

### Windows

SmartScreen may block the installer. Click `More info` -> `Run anyway`.

### Linux

AppImage needs executable permission:

```bash
chmod +x Infimount-*.AppImage
./Infimount-*.AppImage
```

---

## 📄 License

[MIT License](LICENSE) — Copyright © 2026 Infimount Contributors

---

## ⭐ Acknowledgements

- **[Apache OpenDAL](https://opendal.apache.org/)** — Unified storage access layer
- **[Tauri](https://tauri.app/)** — Lightweight native app framework
- **[React](https://react.dev/)** + **[TypeScript](https://www.typescriptlang.org/)** — Modern frontend stack
- **[File Icons](https://github.com/dmhendricks/file-icon-vectors/)** — File Icons by Dan Hendricks

---

<p align="center">
  Made with ❤️ by the Infimount community
</p>
