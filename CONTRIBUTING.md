# Contributing to Infimount

Thank you for your interest in contributing to Infimount! This document provides guidelines and instructions for contributing.

## Table of Contents
- [Code of Conduct](#code-of-conduct)
- [Getting Started](#getting-started)
- [Development Setup](#development-setup)
- [Making Changes](#making-changes)
- [Pull Request Process](#pull-request-process)
- [Code Style](#code-style)
- [Reporting Issues](#reporting-issues)

## Code of Conduct

This project adheres to the [Contributor Covenant Code of Conduct](CODE_OF_CONDUCT.md). By participating, you are expected to uphold this code.

## Getting Started

1. **Fork the repository** on GitHub
2. **Clone your fork** locally:
   ```bash
   git clone https://github.com/YOUR_USERNAME/infimount.git
   cd infimount
   ```
3. **Add the upstream remote**:
   ```bash
   git remote add upstream https://github.com/infimount/infimount.git
   ```

## Development Setup

### Prerequisites
- **Rust** (latest stable via rustup)
- **Node.js** ≥ 18
- **pnpm** (package manager)
- **Tauri dependencies** (see [Tauri Prerequisites](https://tauri.app/v1/guides/getting-started/prerequisites))

### Install Dependencies
```bash
# Frontend dependencies
cd apps/desktop
pnpm install

# Verify Tauri CLI
pnpm tauri --version
```

### Run Development Mode
```bash
cd apps/desktop
pnpm tauri dev
```

### Run Tests
```bash
# Frontend tests
cd apps/desktop
pnpm test

# Rust tests
cargo test --workspace
```

## Making Changes

1. **Create a branch** for your changes:
   ```bash
   git checkout -b feature/your-feature-name
   ```

2. **Make your changes** following our [code style](#code-style) guidelines

3. **Test your changes**:
   ```bash
   pnpm test
   pnpm lint
   ```

4. **Commit your changes** with a clear message:
   ```bash
   git commit -m "feat: add new storage backend support"
   ```
   
   We follow [Conventional Commits](https://www.conventionalcommits.org/):
   - `feat:` - New feature
   - `fix:` - Bug fix
   - `docs:` - Documentation changes
   - `refactor:` - Code refactoring
   - `test:` - Adding or updating tests
   - `chore:` - Maintenance tasks

## Pull Request Process

1. **Update your branch** with the latest upstream changes:
   ```bash
   git fetch upstream
   git rebase upstream/main
   ```

2. **Push your branch** to your fork:
   ```bash
   git push origin feature/your-feature-name
   ```

3. **Open a Pull Request** on GitHub with:
   - Clear title describing the change
   - Description of what and why
   - Link to any related issues

4. **Address review feedback** and update your PR as needed

5. **Ensure CI passes** - all tests and linting must pass

## Code Style

### TypeScript/React
- Use TypeScript strict mode
- Follow ESLint + Prettier configurations
- Use functional components with hooks
- Keep components focused and composable

### Rust
- Follow standard Rust conventions (`cargo fmt`, `cargo clippy`)
- Keep the core crate UI-agnostic
- Delegate storage operations to OpenDAL

### General
- Write clear, self-documenting code
- Add comments for complex logic
- Include tests for new functionality
- Update documentation as needed

## Reporting Issues

When reporting issues, please include:

1. **Description** - What happened vs. what you expected
2. **Steps to reproduce** - Minimal steps to reproduce the issue
3. **Environment** - OS, app version, relevant configuration
4. **Logs/Screenshots** - Any relevant error messages or visual issues

Use the appropriate issue template when available.

---

Thank you for contributing to Infimount!
