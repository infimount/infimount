use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::errors::{err_with_details, map_opendal_error, McpErrorCode, McpResult};
use crate::opendal_adapter;
use crate::path::{enforce_root_operation, parse_mcp_path, resolve_storage_path, FsOp};

use super::common::{collect_entries, FsToolsContext};

fn default_max_results() -> u32 {
    200
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SearchPathsInput {
    pub path: String,
    pub pattern: String,
    #[serde(default = "default_max_results")]
    pub max_results: u32,
}

#[derive(Debug, Serialize)]
pub struct SearchPathsOutput {
    pub path: String,
    pub matches: Vec<String>,
}

pub async fn search_paths(
    ctx: &FsToolsContext,
    input: SearchPathsInput,
) -> McpResult<SearchPathsOutput> {
    if input.max_results == 0 || input.max_results > 2000 {
        return Err(err_with_details(
            McpErrorCode::ERR_INVALID_PATH,
            "max_results must be between 1 and 2000",
            json!({ "max_results": input.max_results }),
        ));
    }

    let parsed = parse_mcp_path(&input.path)?;
    enforce_root_operation(FsOp::SearchPaths, &parsed)?;
    let resolved = resolve_storage_path(&ctx.registry, &parsed.normalized)?;
    let op = opendal_adapter::build_operator(&resolved.storage)?;

    if parsed.backend_path.is_empty() {
        let mut matches = Vec::new();
        if parsed.normalized.contains(&input.pattern) {
            matches.push(parsed.normalized.clone());
        }

        let entries = collect_entries(&op, &resolved.storage.name, "", true).await?;
        matches.extend(
            entries
                .into_iter()
                .map(|entry| entry.path)
                .filter(|path| path.contains(&input.pattern)),
        );
        matches.sort();
        matches.truncate(input.max_results as usize);

        return Ok(SearchPathsOutput {
            path: parsed.normalized,
            matches,
        });
    }

    let meta = op
        .stat(&parsed.backend_path)
        .await
        .map_err(|e| map_opendal_error(&e, McpErrorCode::ERR_INTERNAL))?;
    if !meta.is_dir() {
        return Err(err_with_details(
            McpErrorCode::ERR_NOT_A_DIRECTORY,
            "path is not a directory",
            json!({ "path": parsed.normalized }),
        ));
    }

    let mut matches = Vec::new();
    if parsed.normalized.contains(&input.pattern) {
        matches.push(parsed.normalized.clone());
    }

    let entries = collect_entries(&op, &resolved.storage.name, &parsed.backend_path, true).await?;
    matches.extend(
        entries
            .into_iter()
            .map(|entry| entry.path)
            .filter(|path| path.contains(&input.pattern)),
    );
    matches.sort();
    matches.truncate(input.max_results as usize);

    Ok(SearchPathsOutput {
        path: parsed.normalized,
        matches,
    })
}
