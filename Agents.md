# Agents Guide – OpenHSB (Open Hybrid Storage Browser)

This document is for code assistants (OpenAI, GitHub Copilot, etc.) working on **OpenHSB**.

The goal is to keep contributions **modular, aligned with the architecture**, and **thin on top of Apache OpenDAL**.

---

## 1. High-Level Intent

OpenHSB is a **cross-platform storage browser**:

- Desktop app (Windows, macOS, Linux) using **Tauri + React**.
- Storage abstraction via **Apache OpenDAL** (Rust).
- **Core principle:** *Do not re-implement filesystem/storage logic; always delegate to OpenDAL.*

The backend should mainly:

1. Maintain a **registry of OpenDAL operators** (one per configured “source”).
2. **Forward** calls from the UI to the correct OpenDAL operator.
3. Persist and load configuration (sources, preferences).

---

## 2. Repository Layout

```text
openhsb/
├── crates/
│   ├── core/                        # Shared Rust backend logic (NO UI, NO Tauri)
│   │   ├── src/
│   │   │   ├── lib.rs               # Public API of the core crate
│   │   │   ├── registry.rs          # Operator registry (source_id → Operator)
│   │   │   ├── models.rs            # Source, Entry, SourceKind, error types
│   │   │   ├── operations.rs        # Thin wrappers around OpenDAL APIs
│   │   │   ├── config.rs            # Read/write openhsb config (JSON/TOML/SQLite)
│   │   │   └── util.rs              # Helpers (path utils, conversions)
│   │   └── Cargo.toml
│   │
│   └── bindings/                    # (Future) Mobile bindings
│       ├── android/
│       └── ios/
│
├── apps/
│   ├── desktop/                     # Tauri desktop app
│   │   ├── src/                     # React/TypeScript frontend
│   │   │   ├── app/                 # App shell, layout, routing
│   │   │   ├── components/          # Generic UI components
│   │   │   ├── features/            # Feature-level modules
│   │   │   │   ├── sources/         # Source list, add/edit dialogs
│   │   │   │   ├── explorer/        # File/dir listing, breadcrumbs
│   │   │   │   ├── preview/         # File preview pane
│   │   │   │   └── operations/      # Long-running operations (copy/move)
│   │   │   ├── lib/                 # API client, shared hooks, utilities
│   │   │   ├── types/               # TS types mirrored from Rust models
│   │   │   └── main.tsx
│   │   └── src-tauri/
│   │       ├── src/
│   │       │   ├── main.rs          # Tauri setup & app bootstrap
│   │       │   ├── commands.rs      # Tauri commands → call into openhsb_core
│   │       │   └── state.rs         # AppState (shared registry/config)
│   │       └── Cargo.toml
│   │
│   └── mobile/                      # (Future) Mobile app shell
│       └── ...
│
├── Cargo.toml                       # Rust workspace
└── pnpm-workspace.yaml              # JS/TS workspace
```

---

## 3. Core Architectural Rules (IMPORTANT)

When generating or modifying code, agents MUST follow these rules:

### 3.1 OpenDAL Usage

- **Do NOT implement your own file or storage logic.**
- Always use **OpenDAL’s `Operator`** for:
  - `list`, `lister`
  - `stat`
  - `read` / `reader`
  - `write` / `writer`
  - `delete`
  - `rename`
  - `copy`
  - `presign`
- If a behavior exists in OpenDAL, call it; do not duplicate or wrap it with extra semantics unless absolutely necessary.

### 3.2 Core vs App Responsibilities

**`crates/core` (openhsb_core):**

- Knows **nothing** about Tauri, React, or UI.
- Responsible for:
  - Building OpenDAL operators from `Source` config (`registry.rs`).
  - Exposing high-level functions like:
    - `list_entries(source_id, path)`
    - `read_bytes(source_id, path, offset, length)`
    - `write_bytes(source_id, path, data)`
    - `delete_entry(source_id, path)`
  - Managing config (load/save source list + preferences).
  - Defining shared models (`Source`, `Entry`, `SourceKind`, error types).
- Should be **portable**: usable by CLI, desktop, mobile, or any future app.

**`apps/desktop/src-tauri` (Tauri backend):**

- Pure **bridge layer** between JS and `openhsb_core`.
- Each Tauri command should:
  - Read input params.
  - Call the appropriate `openhsb_core` function.
  - Map errors to strings / basic error types for JS.
- MUST NOT:
  - Contain storage logic.
  - Use OpenDAL directly (that belongs in `crates/core`).
  - Persist config directly (delegate to core).

**`apps/desktop/src` (React frontend):**

- Handles **UI only**:
  - Layout, navigation, dialogs, forms.
  - State management, hooks, UX logic.
- Calls Tauri commands via `invoke(...)` through a thin API client in `lib/api.ts`.
- MUST NOT:
  - Implement any storage logic.
  - Access filesystem directly (except via Tauri APIs when explicitly needed and justified).

---

## 4. How to Add or Modify Backend Features

### 4.1 Adding a New Storage Backend (SourceKind)

**Goal:** Support a new backend (e.g., `SourceKind::S3`, `SourceKind::Webdav`, etc.)

1. **Update models**  
   - Add new variant to `SourceKind`.
   - Document required config keys.

2. **Update registry**  
   - Add builder logic in `build_operator`.

3. **No changes in operations**  
   - Operations remain backend-agnostic.

4. **Update frontend forms**  
   - New fields in add/edit source dialogs.

---

## 5. How to Add or Modify Frontend Features

- Add new TS/React components under proper feature directories.
- Keep logic in hooks.
- Keep UI dumb when possible.
- Use centralized `lib/api.ts` for all backend calls.

---

## 6. Error Handling and Logging

- Core: return structured errors.
- Tauri: map to user-safe messages.
- UI: show toast and developer logs.

---

## 7. Things Agents MUST NOT Do

- ❌ Do not reimplement OpenDAL logic.
- ❌ Do not bypass openhsb_core.
- ❌ Do not mix UI logic into Rust core.
- ❌ Do not change shared models without syncing TS/Rust.
- ❌ Do not create deep couplings between features.

---

## 8. Future Directions (Context Only)

- Mobile client using same backend.
- CLI version.
- Remote web console.
- Rich preview support.

Focus remains:

> **OpenHSB Desktop – clean, thin, multi-backend storage browser.**