use rmcp::model::{AnnotateAble, RawResource, ReadResourceResult, Resource, ResourceContents};

use crate::errors::McpResult;
use crate::path::parse_mcp_path;
use crate::tools_fs::{self, FsToolsContext, ListDirInput, ReadFileInput, StatPathInput};

const RESOURCE_SCHEME: &str = "infimount://";
const ROOT_RESOURCE_URI: &str = "infimount:///";
const RESOURCE_LIST_LIMIT: u32 = 200;
const RESOURCE_FILE_MAX_BYTES: u32 = 262_144;

pub fn root_resource_uri() -> &'static str {
    ROOT_RESOURCE_URI
}

pub fn storage_root_uri(storage_name: &str) -> String {
    format!("{RESOURCE_SCHEME}{storage_name}/")
}

pub fn mcp_path_to_resource_uri(path: &str) -> String {
    if path == "/" {
        return ROOT_RESOURCE_URI.to_string();
    }

    match parse_mcp_path(path) {
        Ok(parsed) if !parsed.is_root => {
            let storage_name = parsed.storage_name.unwrap_or_default();
            if parsed.backend_path.is_empty() {
                storage_root_uri(&storage_name)
            } else {
                format!("{RESOURCE_SCHEME}{storage_name}/{}", parsed.backend_path)
            }
        }
        _ => ROOT_RESOURCE_URI.to_string(),
    }
}

pub fn resource_uri_to_mcp_path(uri: &str) -> Result<String, String> {
    let Some(rest) = uri.strip_prefix(RESOURCE_SCHEME) else {
        return Err("resource URI must start with infimount://".to_string());
    };

    if rest == "/" || rest.is_empty() {
        return Ok("/".to_string());
    }

    let (storage_name, suffix) = match rest.split_once('/') {
        Some((storage_name, suffix)) => (storage_name, suffix),
        None => (rest, ""),
    };

    if storage_name.is_empty() {
        return Err("resource URI is missing a storage name".to_string());
    }

    if suffix.is_empty() {
        Ok(format!("/{storage_name}"))
    } else {
        Ok(format!("/{storage_name}/{suffix}"))
    }
}

pub fn list_resources(ctx: &FsToolsContext) -> McpResult<Vec<Resource>> {
    let mut resources = Vec::new();
    resources.push(
        RawResource::new(root_resource_uri(), "/")
            .with_title("Infimount Root")
            .with_description("Virtual root resource representing '/'.")
            .with_mime_type("application/json")
            .no_annotation(),
    );

    for storage in ctx.registry.list_exposed_enabled()? {
        resources.push(
            RawResource::new(storage_root_uri(&storage.name), storage.name.clone())
                .with_title(storage.name.clone())
                .with_description(format!("Mounted storage root for '{}'.", storage.name))
                .with_mime_type("application/json")
                .no_annotation(),
        );
    }

    Ok(resources)
}

pub async fn read_resource(ctx: &FsToolsContext, uri: &str) -> McpResult<ReadResourceResult> {
    let path = resource_uri_to_mcp_path(uri).map_err(|message| {
        crate::errors::err(crate::errors::McpErrorCode::ERR_INVALID_PATH, message)
    })?;

    let stat = tools_fs::stat_path(ctx, StatPathInput { path: path.clone() }).await?;

    let contents = if stat.entry_type == tools_fs::EntryType::Dir {
        let listing = tools_fs::list_dir(
            ctx,
            ListDirInput {
                path: path.clone(),
                recursive: false,
                limit: RESOURCE_LIST_LIMIT,
                cursor: None,
            },
        )
        .await?;

        vec![ResourceContents::text(
            serde_json::to_string_pretty(&listing).unwrap_or_else(|_| "{}".to_string()),
            uri.to_string(),
        )
        .with_mime_type("application/json")]
    } else {
        match tools_fs::read_file(
            ctx,
            ReadFileInput {
                path: path.clone(),
                offset_bytes: 0,
                max_bytes: RESOURCE_FILE_MAX_BYTES,
                as_text: true,
                encoding: "utf-8".to_string(),
            },
        )
        .await
        {
            Ok(file) => {
                vec![
                    ResourceContents::text(file.content, uri.to_string()).with_mime_type(
                        stat.content_type
                            .unwrap_or_else(|| "text/plain; charset=utf-8".to_string()),
                    ),
                ]
            }
            Err(err) if err.code == crate::errors::McpErrorCode::ERR_TEXT_DECODE_FAILED => {
                let file = tools_fs::read_file(
                    ctx,
                    ReadFileInput {
                        path: path.clone(),
                        offset_bytes: 0,
                        max_bytes: RESOURCE_FILE_MAX_BYTES,
                        as_text: false,
                        encoding: "utf-8".to_string(),
                    },
                )
                .await?;

                vec![
                    ResourceContents::blob(file.content, uri.to_string()).with_mime_type(
                        stat.content_type
                            .unwrap_or_else(|| "application/octet-stream".to_string()),
                    ),
                ]
            }
            Err(err) => return Err(err),
        }
    };

    Ok(ReadResourceResult::new(contents))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::StorageRecord;
    use crate::tools_fs::FsToolsContext;
    use serde_json::json;
    use tempfile::TempDir;

    fn registry_in(dir: &TempDir) -> crate::registry::StorageRegistry {
        crate::registry::StorageRegistry::new(Some(dir.path().join("storages.json")))
    }

    #[test]
    fn resource_uri_round_trip_handles_root_and_storage_paths() {
        assert_eq!(resource_uri_to_mcp_path("infimount:///").unwrap(), "/");
        assert_eq!(
            resource_uri_to_mcp_path("infimount://Local/dir/file.txt").unwrap(),
            "/Local/dir/file.txt"
        );
        assert_eq!(mcp_path_to_resource_uri("/"), "infimount:///");
        assert_eq!(mcp_path_to_resource_uri("/Local"), "infimount://Local/");
    }

    #[tokio::test]
    async fn read_root_resource_returns_json_listing() {
        let dir = TempDir::new().unwrap();
        let registry = registry_in(&dir);
        let storage = StorageRecord::new(
            "Local".to_string(),
            "local".to_string(),
            json!({"root": dir.path()}),
        );
        registry.save_all_atomic(&[storage]).unwrap();
        let ctx = FsToolsContext { registry };

        let result = read_resource(&ctx, "infimount:///").await.unwrap();
        assert_eq!(result.contents.len(), 1);
        let ResourceContents::TextResourceContents { text, .. } = &result.contents[0] else {
            panic!("expected text resource");
        };
        assert!(text.contains("\"entries\""));
        assert!(text.contains("Local"));
    }
}
