use serde_json::json;

pub fn schema_list_dir() -> serde_json::Value {
    json!({
      "type": "object",
      "properties": {
        "path": { "type": "string", "description": "Absolute path, e.g. '/' or '/PhotosS3/trips'." },
        "recursive": { "type": "boolean", "default": false },
        "limit": { "type": "integer", "default": 200, "minimum": 1, "maximum": 1000 },
        "cursor": { "type": "string" }
      },
      "required": ["path"],
      "additionalProperties": false
    })
}

pub fn schema_stat_path() -> serde_json::Value {
    json!({
      "type": "object",
      "properties": {
        "path": { "type": "string", "description": "Absolute path, e.g. '/' or '/PhotosS3/trips/a.txt'." }
      },
      "required": ["path"],
      "additionalProperties": false
    })
}

pub fn schema_read_file() -> serde_json::Value {
    json!({
      "type": "object",
      "properties": {
        "path": { "type": "string", "description": "Absolute file path." },
        "offset_bytes": { "type": "integer", "default": 0, "minimum": 0 },
        "max_bytes": { "type": "integer", "default": 262144, "minimum": 1, "maximum": 2097152 },
        "as_text": { "type": "boolean", "default": true },
        "encoding": { "type": "string", "default": "utf-8", "description": "Text encoding. Only utf-8 is supported in v1." }
      },
      "required": ["path"],
      "additionalProperties": false
    })
}

pub fn schema_mkdir() -> serde_json::Value {
    json!({
      "type": "object",
      "properties": {
        "path": { "type": "string", "description": "Absolute directory path." },
        "parents": { "type": "boolean", "default": true },
        "exist_ok": { "type": "boolean", "default": true }
      },
      "required": ["path"],
      "additionalProperties": false
    })
}

pub fn schema_write_file() -> serde_json::Value {
    json!({
      "type": "object",
      "properties": {
        "path": { "type": "string", "description": "Absolute file path." },
        "content": { "type": "string", "description": "Text content. Only utf-8 encoding is supported in v1." },
        "encoding": { "type": "string", "default": "utf-8", "description": "Text encoding. Only utf-8 is supported in v1." },
        "overwrite": { "type": "boolean", "default": true },
        "create_parents": { "type": "boolean", "default": false }
      },
      "required": ["path", "content"],
      "additionalProperties": false
    })
}

pub fn schema_delete_path() -> serde_json::Value {
    json!({
      "type": "object",
      "properties": {
        "path": { "type": "string", "description": "Absolute file or directory path." },
        "recursive": { "type": "boolean", "default": false }
      },
      "required": ["path"],
      "additionalProperties": false
    })
}

pub fn schema_copy_path() -> serde_json::Value {
    json!({
      "type": "object",
      "properties": {
        "src": { "type": "string", "description": "Absolute source path." },
        "dst": { "type": "string", "description": "Absolute destination path." },
        "overwrite": { "type": "boolean", "default": false },
        "recursive": { "type": "boolean", "default": false }
      },
      "required": ["src", "dst"],
      "additionalProperties": false
    })
}

pub fn schema_move_path() -> serde_json::Value {
    json!({
      "type": "object",
      "properties": {
        "src": { "type": "string", "description": "Absolute source file path." },
        "dst": { "type": "string", "description": "Absolute destination file path." },
        "overwrite": { "type": "boolean", "default": false }
      },
      "required": ["src", "dst"],
      "additionalProperties": false
    })
}

pub fn schema_search_paths() -> serde_json::Value {
    json!({
      "type": "object",
      "properties": {
        "path": { "type": "string", "description": "Absolute directory path." },
        "pattern": { "type": "string", "description": "Case-sensitive substring match." },
        "max_results": { "type": "integer", "default": 200, "minimum": 1, "maximum": 2000 }
      },
      "required": ["path", "pattern"],
      "additionalProperties": false
    })
}

pub fn schema_generate_download_link() -> serde_json::Value {
    json!({
      "type": "object",
      "properties": {
        "path": { "type": "string", "description": "Absolute file path." },
        "expires_seconds": { "type": "integer", "default": 900, "minimum": 60, "maximum": 86400 }
      },
      "required": ["path"],
      "additionalProperties": false
    })
}

pub fn schema_list_storages() -> serde_json::Value {
    json!({
      "type": "object",
      "properties": {},
      "additionalProperties": false
    })
}

pub fn schema_add_storage() -> serde_json::Value {
    json!({
      "type": "object",
      "properties": {
        "name": { "type": "string", "maxLength": 64 },
        "backend": { "type": "string" },
        "config": { "type": "object" },
        "enabled": { "type": "boolean", "default": true },
        "mcp_exposed": { "type": "boolean", "default": true },
        "read_only": { "type": "boolean", "default": false }
      },
      "required": ["name", "backend", "config"],
      "additionalProperties": false
    })
}

pub fn schema_edit_storage() -> serde_json::Value {
    json!({
      "type": "object",
      "properties": {
        "name": { "type": "string" },
        "patch": {
          "type": "object",
          "properties": {
            "backend": { "type": "string" },
            "config": { "type": "object" },
            "enabled": { "type": "boolean" },
            "mcp_exposed": { "type": "boolean" },
            "read_only": { "type": "boolean" },
            "new_name": { "type": "string", "maxLength": 64 }
          },
          "additionalProperties": false
        }
      },
      "required": ["name", "patch"],
      "additionalProperties": false
    })
}

pub fn schema_remove_storage() -> serde_json::Value {
    json!({
      "type": "object",
      "properties": {
        "name": { "type": "string" }
      },
      "required": ["name"],
      "additionalProperties": false
    })
}

pub fn schema_import_config() -> serde_json::Value {
    json!({
      "type": "object",
      "properties": {
        "json": { "type": "string" },
        "mode": { "type": "string", "default": "merge" },
        "on_conflict": { "type": "string", "default": "error" }
      },
      "required": ["json"],
      "additionalProperties": false
    })
}

pub fn schema_export_config() -> serde_json::Value {
    json!({
      "type": "object",
      "properties": {
        "include_secrets": { "type": "boolean", "default": false }
      },
      "additionalProperties": false
    })
}

pub fn schema_validate_storage() -> serde_json::Value {
    json!({
      "type": "object",
      "properties": {
        "name": { "type": "string" }
      },
      "required": ["name"],
      "additionalProperties": false
    })
}
