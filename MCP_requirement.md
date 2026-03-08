# Infimount MCP Server Feature Requirements and Implementation Notes

## 0. Objective

Add an MCP server to Infimount so any MCP client can browse and operate on all configured storages using filesystem-like paths. All MCP filesystem operations must accept a single `path` parameter (no explicit storage parameter). The first path segment is treated as the mounted storage name. The MCP layer uses storage names and paths only. Infimount keeps internal stable IDs for storages, but MCP never exposes or requires IDs.

---

## 1. Scope

### In scope

1. MCP server implemented in Rust using the official MCP Rust SDK (`rmcp`).
2. Virtual filesystem namespace:

   * Root path `/` lists configured storages that are `enabled=true` and `mcp_exposed=true`.
   * Paths like `/PhotosS3/trips/2024/img.jpg` route to the `PhotosS3` storage and backend path `trips/2024/img.jpg`.
3. Core filesystem-like MCP tools:

   * `list_dir`
   * `stat_path`
   * `read_file`
   * `write_file`
   * `mkdir`
   * `move_path`
   * `copy_path`
   * `delete_path`
4. Storage management MCP tools:

   * `list_storages`
   * `add_storage`
   * `edit_storage`
   * `remove_storage`
   * `import_config`
   * `export_config`
   * `validate_storage`
5. Optional utility MCP tools:

   * `search_paths` (best-effort recursive)
   * `generate_download_link` (only when backend supports presign read)
6. Storage registry persistence (local file), including:

   * internal `id` (UUID)
   * `name` (unique, user-facing identifier)
   * backend type
   * config JSON (including credentials, masked in responses by default)
   * flags: enabled, mcp_exposed, read_only
7. Deterministic error model: structured error codes, no ambiguous failures.

### Out of scope (explicitly not implemented)

* Multi-user, RBAC, roles, teams.
* External secret vaults, secret references, multiple edit modes.
* Version-aware operations.
* Per-path allowlists or deny lists beyond `mcp_exposed` and `read_only`.
* Background indexing for search.
* Remote hosted multi-tenant MCP gateway.

---

## 2. Definitions and Terminology

* **Storage**: a configured backend (S3, local filesystem, WebDAV, etc.) exposed via OpenDAL.
* **Mount name**: the user-defined storage `name`. Used as the top-level directory under `/`.
* **Virtual root**: path `/` which lists mount names.
* **Backend path**: path inside the storage after removing the mount prefix.
* **MCP path**: filesystem-style string beginning with `/`.

---

## 3. Functional Requirements

### 3.1 Virtual Root Behavior

1. `list_dir("/")` returns the list of storages where:

   * `enabled == true`
   * `mcp_exposed == true`
2. The returned entries at `/` must be directories.
3. Any operation other than `list_dir("/")` and `stat_path("/")` must fail with `ERR_ROOT_OPERATION_NOT_ALLOWED`.

### 3.2 Path Parsing and Routing

A single deterministic parsing function must be used by all filesystem tools.

#### Rules

Given input `path`:

1. `path` must be an absolute path and start with `/`.
2. Normalize:

   * collapse multiple slashes: `//` -> `/`
   * remove trailing slash except if path is exactly `/`
3. If normalized path == `/`:

   * mount is `None`
   * backend path is `None`
4. Else:

   * split by `/` ignoring the first empty segment.
   * first segment is `storage_name`.
   * remaining segments joined by `/` is `backend_path` (may be empty string meaning storage root).

Examples:

* `/` -> mount none
* `/PhotosS3` -> storage_name `PhotosS3`, backend_path `""`
* `/PhotosS3/` -> normalize to `/PhotosS3`, backend_path `""`
* `/PhotosS3/trips/2024/a.jpg` -> backend_path `trips/2024/a.jpg`

#### Storage lookup

* `storage_name` must match storage registry `name` exactly (case-sensitive).
* If not found -> `ERR_STORAGE_NOT_FOUND`.
* If found but `enabled=false` -> `ERR_STORAGE_DISABLED`.
* If found but `mcp_exposed=false` -> `ERR_STORAGE_NOT_EXPOSED`.

### 3.3 Storage Registry Requirements

#### Storage object schema (internal)

* `id`: UUID v4 string, immutable.
* `name`: string, unique, user-visible mount name.
* `backend`: string, one of supported backend identifiers (must match OpenDAL service name mapping in code).
* `config`: JSON object (backend-specific).
* `enabled`: boolean.
* `mcp_exposed`: boolean.
* `read_only`: boolean.
* `created_at`: ISO-8601 UTC string.
* `updated_at`: ISO-8601 UTC string.

#### Name uniqueness constraints

* Unique by exact string match.
* Empty string is invalid.
* `/` is invalid.
* Name must not contain path separator `/`.
* Leading/trailing whitespace must be trimmed on write.
* Maximum length: 64 characters.
* If violates -> `ERR_INVALID_STORAGE_NAME` or `ERR_STORAGE_NAME_CONFLICT`.

#### Persistence

* Stored as a single JSON file at:

  * Linux/macOS: `$XDG_CONFIG_HOME/infimount/storages.json` if set else `~/.config/infimount/storages.json`
  * Windows: `%APPDATA%\infimount\storages.json`
* Writes must be atomic:

  * write to temp file then rename.
* Must support concurrent access from UI and MCP server:

  * use a file lock (advisory lock) or a simple single-process invariant if MCP server runs in the same process.
  * If lock cannot be acquired within 2 seconds -> `ERR_REGISTRY_LOCK_TIMEOUT`.

### 3.4 OpenDAL Operator Construction

* For each request requiring backend access:

  1. load storage entry
  2. build OpenDAL `Operator` from `(backend, config)`
  3. execute operation
* Operator caching is allowed but must be safe with config edits:

  * cache key: storage `id` plus `updated_at` timestamp
  * on mismatch, rebuild operator

### 3.5 Filesystem Tools Requirements

All filesystem tools use `path` strings only.

#### 3.5.1 `list_dir`

Input:

* `path`: string absolute
* `recursive`: boolean default false
* `limit`: integer default 200, min 1, max 1000
* `cursor`: optional string for pagination

Behavior:

* If `path == "/"`: list mount directories (see 3.1)
* Else:

  * route to storage and list `backend_path` as a directory.
  * If `backend_path` is empty, list storage root.
* Must return entries with:

  * `name` (base name)
  * `path` (full path starting with `/storage_name/...`)
  * `type`: `file` | `dir`
  * `size_bytes` (files only, nullable if unknown)
  * `modified_at` (nullable)
  * `etag` (nullable)
* Pagination:

  * If OpenDAL backend supports pagination, use it.
  * Else implement in-memory pagination by listing and slicing, but still return deterministic ordering.
* Ordering:

  * directories first, then files
  * within each group sort lexicographically by `name` (Unicode code point order)
* Errors:

  * If path does not exist -> `ERR_PATH_NOT_FOUND`
  * If path is not a directory -> `ERR_NOT_A_DIRECTORY`

#### 3.5.2 `stat_path`

Input:

* `path`: string absolute

Behavior:

* If `path == "/"`:

  * return type `dir`
  * return listable true
* Else:

  * OpenDAL `stat` backend_path
  * map metadata to:

    * `type`
    * `size_bytes` (nullable)
    * `modified_at` (nullable)
    * `etag` (nullable)
    * `content_type` (nullable)
* Errors:

  * not found -> `ERR_PATH_NOT_FOUND`

#### 3.5.3 `read_file`

Input:

* `path`: string absolute
* `offset_bytes`: integer default 0 min 0
* `max_bytes`: integer default 262144 (256 KiB), min 1, max 2097152 (2 MiB)
* `as_text`: boolean default true
* `encoding`: string default "utf-8" (only applied if as_text)

Behavior:

* Reject if `path == "/"` -> `ERR_ROOT_OPERATION_NOT_ALLOWED`
* Must stat first. If directory -> `ERR_IS_A_DIRECTORY`
* Read a range:

  * If backend supports ranged read, use offset and length.
  * Else read full then slice, but cap total read to `max_bytes` and stop early if possible.
* Output:

  * `content`: string if `as_text=true` else base64 string
  * `truncated`: boolean
  * `read_bytes`: integer
* Text decoding:

  * If decoding fails -> `ERR_TEXT_DECODE_FAILED` and include `hint` “use as_text=false”

#### 3.5.4 `write_file`

Input:

* `path`: string absolute
* `content`: string (text)
* `encoding`: string default "utf-8"
* `overwrite`: boolean default true
* `create_parents`: boolean default false

Behavior:

* Reject if storage `read_only=true` -> `ERR_STORAGE_READ_ONLY`
* Reject if `path == "/"` -> `ERR_ROOT_OPERATION_NOT_ALLOWED`
* If `overwrite=false` and file exists -> `ERR_ALREADY_EXISTS`
* Parent creation:

  * If `create_parents=true`, ensure all parent directories exist, create them.
  * If false and parent missing -> `ERR_PARENT_NOT_FOUND`
* Writes are atomic when backend supports it. If not, best effort.
* Errors:

  * if path points to directory -> `ERR_IS_A_DIRECTORY`
  * permission denied -> `ERR_PERMISSION_DENIED`

#### 3.5.5 `mkdir`

Input:

* `path`: string absolute
* `parents`: boolean default true
* `exist_ok`: boolean default true

Behavior:

* Reject if storage `read_only=true` -> `ERR_STORAGE_READ_ONLY`
* Reject if `path == "/"` -> `ERR_ROOT_OPERATION_NOT_ALLOWED`
* Ensure directory exists.
* If `exist_ok=false` and exists -> `ERR_ALREADY_EXISTS`

#### 3.5.6 `move_path`

Input:

* `src`: string absolute
* `dst`: string absolute
* `overwrite`: boolean default false

Behavior:

* Reject if root involved -> `ERR_ROOT_OPERATION_NOT_ALLOWED`
* Resolve src and dst storage names:

  * If different storages: must perform copy then delete (cross-storage move).
  * If same storage:

    * Use OpenDAL rename if supported.
    * Else copy then delete.
* If destination exists:

  * if `overwrite=false` -> `ERR_ALREADY_EXISTS`
  * if overwrite true -> delete destination then move.
* Reject if source storage is read_only OR destination storage is read_only -> `ERR_STORAGE_READ_ONLY`

#### 3.5.7 `copy_path`

Input:

* `src`: string absolute
* `dst`: string absolute
* `overwrite`: boolean default false
* `recursive`: boolean default false

Behavior:

* Same storage:

  * Use OpenDAL copy if supported for file objects.
  * For directories, if recursive=true, enumerate and copy tree.
* Cross storage:

  * Stream read from src and write to dst.
  * For large files, chunked stream with fixed chunk size 8 MiB.
* If source is directory and recursive=false -> `ERR_IS_A_DIRECTORY`
* Destination exists and overwrite=false -> `ERR_ALREADY_EXISTS`
* Reject if destination storage read_only=true -> `ERR_STORAGE_READ_ONLY`

#### 3.5.8 `delete_path`

Input:

* `path`: string absolute
* `recursive`: boolean default false

Behavior:

* Reject if storage read_only=true -> `ERR_STORAGE_READ_ONLY`
* Reject if `path == "/"` -> `ERR_ROOT_OPERATION_NOT_ALLOWED`
* If path is file: delete file
* If path is directory:

  * if recursive=false -> `ERR_NOT_EMPTY_OR_DIR`
  * if recursive=true: list recursively then delete children then directory if supported
* Must be safe and deterministic.

### 3.6 Utility Tools

#### 3.6.1 `search_paths`

Input:

* `path`: string absolute (directory)
* `pattern`: string (substring match, case-sensitive)
* `max_results`: integer default 200, min 1, max 2000

Behavior:

* Best-effort recursive traversal from `path`.
* Return list of matching full paths.
* Deterministic order: lexicographic by full path.

#### 3.6.2 `generate_download_link`

Input:

* `path`: string absolute
* `expires_seconds`: integer default 900, min 60, max 86400

Behavior:

* Only for backends that support presign read.
* If not supported -> `ERR_PRESIGN_NOT_SUPPORTED`
* Return:

  * `url`: string
  * `expires_at`: ISO-8601 UTC

---

## 4. Storage Management Tools Requirements

These tools manage the registry. They are separate from filesystem ops and do not use filesystem paths.

### 4.1 `list_storages`

Returns all storages in registry, including those not exposed to MCP.
Response must mask secrets by default.

Masking rules:

* For keys matching any of: `secret`, `password`, `token`, `access_key`, `secret_key`, `client_secret`, `session_token` (case-insensitive contains):

  * replace value with `"********"`
* Keep non-secret config values intact.

### 4.2 `add_storage`

Input:

* `name`
* `backend`
* `config` (JSON object)
* `enabled` default true
* `mcp_exposed` default true
* `read_only` default false

Behavior:

* Validate name constraints and uniqueness.
* Create UUID.
* Save with timestamps.
* Return created storage record with masked secrets.

### 4.3 `edit_storage`

Input:

* `name` (existing storage name)
* `patch` object:

  * may update: `backend`, `config`, `enabled`, `mcp_exposed`, `read_only`, and optionally `new_name`
* If renaming, validate uniqueness.
* Update `updated_at`.
* Return updated record masked.

### 4.4 `remove_storage`

Input:

* `name`
* Behavior:

  * Remove from registry.
  * Return `{ removed: true }`
* If not found -> `ERR_STORAGE_NOT_FOUND`

### 4.5 `import_config`

Input:

* `json`: full registry JSON content OR list of storage objects (support both)
* `mode`: `"merge"` | `"replace"` default `"merge"`
* `on_conflict`: `"error"` | `"overwrite"` | `"rename"` default `"error"`

Behavior:

* Validate all entries.
* `replace`: replace entire registry file atomically.
* `merge`: add or update based on storage `name` and conflict mode.
* `rename` conflict resolution:

  * append ` (2)`, ` (3)` etc until unique.

### 4.6 `export_config`

Input:

* `include_secrets`: boolean default false

Behavior:

* If include_secrets=false: secret fields must be masked exactly as `"********"`.
* Return JSON string of storages array.

### 4.7 `validate_storage`

Input:

* `name`

Behavior:

* Build operator.
* Perform minimal validation:

  * `stat` storage root or list root depending on backend
  * Must succeed within timeout 10 seconds.
* Return:

  * `valid`: boolean
  * `details`: string
  * `capabilities`: summarized boolean map derived from OpenDAL capability:

    * list, stat, read, write, delete, copy, rename, presign_read, create_dir

---

## 5. MCP Server Requirements

### 5.1 Transport

* Support stdio transport at minimum.
* CLI option `--transport stdio` default.
* No network listeners in v1.

### 5.2 Tool Naming

Tool names must be exactly:

* `list_dir`
* `stat_path`
* `read_file`
* `write_file`
* `mkdir`
* `move_path`
* `copy_path`
* `delete_path`
* `search_paths`
* `generate_download_link`
* `list_storages`
* `add_storage`
* `edit_storage`
* `remove_storage`
* `import_config`
* `export_config`
* `validate_storage`

No aliases. No spaces. Lower snake case.

### 5.3 Tool Schemas

Each tool must have an explicit JSON schema with:

* required fields
* defaults
* min and max constraints
* clear descriptions
  This is non-optional.

### 5.4 Resources

Expose resources for browsing and file reads:

* Resource URI format: `infimount://<storage_name>/<path>`
* Root resource: `infimount:///` representing `/`
  Resource handlers:
* Listing resource for directories should call `list_dir`.
* File resource should call `read_file` with safe default max bytes.

### 5.5 Prompts

Provide minimal prompts (optional but included):

* `browse_storage`: asks the model to list `/` then navigate into a selected storage.
* `summarize_file`: reads a file and summarizes.

Prompts must be deterministic templates with parameters.

---

## 6. Error Model

All tool failures must return a structured error object:

```json
{
  "ok": false,
  "error": {
    "code": "ERR_STORAGE_NOT_FOUND",
    "message": "Storage 'PhotosS3' not found",
    "details": { "path": "/PhotosS3/x.txt" }
  }
}
```

Success responses must be:

```json
{ "ok": true, "data": { ... } }
```

### Required error codes

* `ERR_INVALID_PATH`
* `ERR_ROOT_OPERATION_NOT_ALLOWED`
* `ERR_STORAGE_NOT_FOUND`
* `ERR_STORAGE_DISABLED`
* `ERR_STORAGE_NOT_EXPOSED`
* `ERR_STORAGE_READ_ONLY`
* `ERR_INVALID_STORAGE_NAME`
* `ERR_STORAGE_NAME_CONFLICT`
* `ERR_PATH_NOT_FOUND`
* `ERR_NOT_A_DIRECTORY`
* `ERR_IS_A_DIRECTORY`
* `ERR_PARENT_NOT_FOUND`
* `ERR_ALREADY_EXISTS`
* `ERR_PERMISSION_DENIED`
* `ERR_TEXT_DECODE_FAILED`
* `ERR_PRESIGN_NOT_SUPPORTED`
* `ERR_REGISTRY_LOCK_TIMEOUT`
* `ERR_BACKEND_UNSUPPORTED`
* `ERR_INTERNAL`

Mapping rules:

* Convert OpenDAL errors into the closest code with a stable mapping table.
* Never expose raw stack traces in `message`.
* Put backend specifics into `details.backend_error` if needed.

---

## 7. Non-Functional Requirements

### 7.1 Performance

* `list_dir` default limit 200 and must not load entire huge listings into memory if backend supports pagination.
* `read_file` max 2 MiB.
* `copy_path` streaming chunk size 8 MiB.
* `validate_storage` timeout 10 seconds.

### 7.2 Safety

* `read_only` enforced on all mutating filesystem tools.
* root operations restricted.
* `export_config(include_secrets=false)` must mask secrets.
* `list_storages` and all storage management responses must mask secrets.

### 7.3 Determinism

* Sorted outputs for directory listing and search.
* Stable JSON field ordering is not required, but content must be deterministic.

### 7.4 Logging

* Log at info level:

  * tool invoked
  * resolved storage name
  * operation type
  * success or error code
  * latency
* Never log secret values.

---

## 8. Implementation Plan (Concrete Steps)

### Step 1: Create crate structure

* `infimount_mcp` crate (or module) with:

  * `registry.rs` (storage registry)
  * `path.rs` (path normalization and routing)
  * `opendal.rs` (operator builder + cache)
  * `errors.rs` (error codes + mapping)
  * `tools_fs.rs` (filesystem tools)
  * `tools_storage.rs` (storage CRUD tools)
  * `resources.rs` (resource handlers)
  * `server.rs` (rmcp server setup)
  * `main.rs` (CLI entry)

### Step 2: Storage registry

* Implement load/save, atomic write, lock.
* Implement validation of storage names.
* Implement secret masking utility:

  * recursive walk JSON and mask any keys matching secret patterns.

### Step 3: Path router

* Implement `normalize_path(path: &str) -> Result<String, Error>`
* Implement `resolve(path: &str) -> ResolvedPath { is_root, storage_name, backend_path }`
* Implement `get_storage(storage_name) -> StorageEntry` with checks.

### Step 4: OpenDAL adapter

* Map `backend` string to OpenDAL service builder.
* Build `Operator` from `config` JSON.
* Implement operator cache keyed by `(storage_id, updated_at)`.

### Step 5: Tools

* Implement all filesystem tools with the rules in section 3.5.
* Implement storage management tools with rules in section 4.
* Ensure every tool uses `{ok,data}` / `{ok,error}` envelopes.

### Step 6: MCP server wiring

* Implement rmcp server:

  * register tools with schemas
  * implement resource handlers
  * implement prompts
* Default transport stdio.

### Step 7: Test suite

Unit tests:

* path normalization and routing
* name validation
* secret masking
* error mapping
* root behavior

Integration tests:

* Local filesystem backend: create temp dir, add storage, run list/read/write/delete.
* Mock S3 optional if available, otherwise skip.

---

## 9. Tool Input and Output Schemas (Exact)

### 9.1 `list_dir`

Input:

```json
{
  "type": "object",
  "properties": {
    "path": { "type": "string", "description": "Absolute path, e.g. '/' or '/PhotosS3/trips'." },
    "recursive": { "type": "boolean", "default": false },
    "limit": { "type": "integer", "default": 200, "minimum": 1, "maximum": 1000 },
    "cursor": { "type": "string" }
  },
  "required": ["path"],
  "additionalProperties": false
}
```

Output data:

```json
{
  "path": "string",
  "entries": [
    {
      "name": "string",
      "path": "string",
      "type": "file|dir",
      "size_bytes": "integer|null",
      "modified_at": "string|null",
      "etag": "string|null"
    }
  ],
  "next_cursor": "string|null"
}
```

### 9.2 `stat_path`

Input: `{ "path": "string" }`
Output data:

```json
{
  "path": "string",
  "type": "file|dir",
  "size_bytes": "integer|null",
  "modified_at": "string|null",
  "etag": "string|null",
  "content_type": "string|null"
}
```

### 9.3 `read_file`

Input:

```json
{
  "type": "object",
  "properties": {
    "path": { "type": "string" },
    "offset_bytes": { "type": "integer", "default": 0, "minimum": 0 },
    "max_bytes": { "type": "integer", "default": 262144, "minimum": 1, "maximum": 2097152 },
    "as_text": { "type": "boolean", "default": true },
    "encoding": { "type": "string", "default": "utf-8" }
  },
  "required": ["path"],
  "additionalProperties": false
}
```

Output data:

```json
{
  "path": "string",
  "content": "string",
  "truncated": "boolean",
  "read_bytes": "integer"
}
```

### 9.4 `write_file`

Input:

```json
{
  "type": "object",
  "properties": {
    "path": { "type": "string" },
    "content": { "type": "string" },
    "encoding": { "type": "string", "default": "utf-8" },
    "overwrite": { "type": "boolean", "default": true },
    "create_parents": { "type": "boolean", "default": false }
  },
  "required": ["path", "content"],
  "additionalProperties": false
}
```

Output data: `{ "path": "string", "written_bytes": "integer" }`

### 9.5 `mkdir`

Input: `{ "path": "string", "parents": boolean, "exist_ok": boolean }` with defaults true.
Output data: `{ "path": "string", "created": "boolean" }`

### 9.6 `move_path`

Input: `{ "src": "string", "dst": "string", "overwrite": boolean }`
Output data: `{ "src": "string", "dst": "string", "moved": "boolean" }`

### 9.7 `copy_path`

Input: `{ "src": "string", "dst": "string", "overwrite": boolean, "recursive": boolean }`
Output data: `{ "src": "string", "dst": "string", "copied": "boolean" }`

### 9.8 `delete_path`

Input: `{ "path": "string", "recursive": boolean }`
Output data: `{ "path": "string", "deleted": "boolean" }`

### 9.9 `search_paths`

Input: `{ "path": "string", "pattern": "string", "max_results": integer }`
Output data: `{ "path": "string", "pattern": "string", "matches": ["string"] }`

### 9.10 `generate_download_link`

Input: `{ "path": "string", "expires_seconds": integer }`
Output data: `{ "path": "string", "url": "string", "expires_at": "string" }`

### 9.11 Storage management schemas

* `list_storages`: no input, returns array of masked storage records.
* `add_storage`: fields in section 4.2.
* `edit_storage`: fields in section 4.3.
* `remove_storage`: `{ "name": "string" }`
* `import_config`: `{ "json": "string|object", "mode": "merge|replace", "on_conflict": "error|overwrite|rename" }`
* `export_config`: `{ "include_secrets": boolean }`
* `validate_storage`: `{ "name": "string" }`

All must set `additionalProperties=false`.

---

## 10. Acceptance Criteria

1. MCP client can call `list_dir("/")` and see all exposed storages as directories.
2. MCP client can call `list_dir("/<storage>")` and see backend listing.
3. MCP client can read and write a file under local backend storage.
4. `read_only=true` blocks all mutations with `ERR_STORAGE_READ_ONLY`.
5. Storage CRUD functions work and persist.
6. Secret masking works in all management outputs and in exported configs when `include_secrets=false`.
7. Errors always return stable `code` and do not leak secrets or stack traces.

---

## 11. Implementation Constraints

* Rust only.
* Use `rmcp` for MCP server.
* Use OpenDAL for storage backends.
* No placeholders, no partially implemented tool schemas.
* All outputs must follow `{ ok: true, data }` / `{ ok: false, error }`.

---

If you want, I can also provide a ready-to-copy Rust file layout with exact function signatures and module skeletons aligned to `rmcp` conventions.
