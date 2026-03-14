use serde::{Deserialize, Serialize};

use crate::errors::McpResult;
use crate::tools_fs::FsToolsContext;

use super::common::masked;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ListStoragesInput {}

#[derive(Debug, Serialize)]
pub struct ListStoragesOutput {
    pub storages: Vec<crate::registry::StorageRecord>,
}

pub async fn list_storages(ctx: &FsToolsContext) -> McpResult<ListStoragesOutput> {
    let storages = ctx
        .registry
        .load_all()?
        .into_iter()
        .map(|storage| masked(&storage))
        .collect();

    Ok(ListStoragesOutput { storages })
}
