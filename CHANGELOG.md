# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.0] - 2026-03-01

First stable release of Infimount — a unified desktop storage browser powered by Apache OpenDAL.

### Features

- **Unified file browser** for local filesystem, Amazon S3, Azure Blob Storage, Google Cloud Storage, and WebDAV
- **Grid and list views** with smooth transitions
- **File preview panel** with inline image, text, and document rendering
- **Drag-and-drop uploads** into any storage backend
- **Drag-select** for multi-file operations
- **Create folder and file** from the UI
- **Storage sidebar** with multiple source management and reordering
- **Verify storage connection** button for validating backend credentials
- **Cross-platform desktop app** — native builds for Linux (.deb, .rpm, .AppImage), macOS (.dmg), and Windows (.msi, .exe)
- **Custom window chrome** with transparent titlebar on macOS
- **System tray** integration with quit menu

### Architecture

- **Rust core** (`infimount_core`) with OpenDAL for backend-agnostic storage operations
- **Tauri 2** bridge layer connecting React frontend to Rust backend
- **React 19 + TypeScript** frontend with Radix UI components
- **IndexMap-based** source registry preserving insertion order (newest-first)
- **Local-first config** — credentials stored on your machine, no cloud dependency

### Infrastructure

- GitHub Actions **CI pipeline** with `cargo fmt`, `clippy`, `cargo test`, ESLint, and TypeScript checks
- **Multi-platform release workflow** with smoke tests, SHA256 checksums, SBOM (SPDX), and build provenance attestation
- Optional **macOS code signing** and **Windows code signing** when secrets are configured
- **Dependabot** for automated dependency updates (Cargo, npm, GitHub Actions)
- **Governance documentation** — GOVERNANCE.md, MAINTAINERS.md, CODEOWNERS, SECURITY.md
- **Homebrew tap** available at `infimount/infimount`

---

[Unreleased]: https://github.com/infimount/infimount/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/infimount/infimount/releases/tag/v0.1.0
