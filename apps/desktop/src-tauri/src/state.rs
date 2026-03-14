use infimount_core::{config, CoreError, Source, SourceKind};
use infimount_mcp::errors::{err_with_details, McpError, McpErrorCode, McpResult};
use infimount_mcp::opendal_adapter::build_operator;
use infimount_mcp::registry::{StorageRecord, StorageRegistry};
use infimount_mcp::runtime::{
    start_http_server_from_settings, McpHttpServerHandle, HTTP_ENDPOINT_PATH,
};
use infimount_mcp::settings::{McpSettings, McpSettingsStore, McpTransport};
use infimount_mcp::tools_fs::FsToolsContext;
use opendal::Operator;
use serde::Serialize;
use serde_json::{json, Map, Value};
use tokio::sync::Mutex;

pub struct AppState {
    pub registry: StorageRegistry,
    pub settings_store: McpSettingsStore,
    http_runtime: Mutex<Option<McpHttpServerHandle>>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpRuntimeStatus {
    pub settings: McpSettings,
    pub running_http: bool,
    pub endpoint: Option<String>,
    pub endpoint_display: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct McpClientSnippets {
    pub stdio: String,
    pub http: String,
}

impl AppState {
    pub fn new() -> McpResult<Self> {
        let registry = StorageRegistry::new(None);
        migrate_legacy_sources_if_needed(&registry)?;

        Ok(Self {
            registry,
            settings_store: McpSettingsStore::new(None),
            http_runtime: Mutex::new(None),
        })
    }

    pub fn fs_context(&self) -> FsToolsContext {
        FsToolsContext {
            registry: self.registry.clone(),
        }
    }

    pub fn list_storages(&self) -> McpResult<Vec<StorageRecord>> {
        self.registry.load_all()
    }

    pub fn find_storage_by_id(&self, storage_id: &str) -> McpResult<StorageRecord> {
        self.registry
            .load_all()?
            .into_iter()
            .find(|storage| storage.id == storage_id)
            .ok_or_else(|| {
                err_with_details(
                    McpErrorCode::ERR_STORAGE_NOT_FOUND,
                    format!("storage '{storage_id}' not found"),
                    json!({ "storage_id": storage_id }),
                )
            })
    }

    pub fn operator_for_storage_id(&self, storage_id: &str) -> Result<Operator, CoreError> {
        let storage = self
            .find_storage_by_id(storage_id)
            .map_err(mcp_error_to_core_error)?;
        build_operator(&storage).map_err(mcp_error_to_core_error)
    }

    pub async fn apply_mcp_settings(&self, settings: McpSettings) -> McpResult<McpRuntimeStatus> {
        self.settings_store.save_atomic(&settings)?;
        self.mcp_status().await
    }

    pub async fn start_http_server(&self) -> McpResult<McpRuntimeStatus> {
        let settings = self.settings_store.load()?;
        if settings.transport != McpTransport::Http {
            return Err(err_with_details(
                McpErrorCode::ERR_INTERNAL,
                "MCP transport is not set to HTTP",
                json!({ "transport": settings.transport }),
            ));
        }

        self.stop_http_server_inner().await?;
        let server = start_http_server_from_settings(self.registry.clone(), &settings)
            .await
            .map_err(map_runtime_io_error)?;
        let endpoint = server.endpoint().to_string();

        let mut guard = self.http_runtime.lock().await;
        *guard = Some(server);
        drop(guard);

        self.status_with_endpoint(settings, Some(endpoint))
    }

    pub async fn stop_http_server(&self) -> McpResult<McpRuntimeStatus> {
        let settings = self.settings_store.load()?;
        self.stop_http_server_inner().await?;
        self.status_with_endpoint(settings, None)
    }

    pub async fn ensure_runtime_from_settings(&self) -> McpResult<()> {
        let settings = self.settings_store.load()?;
        if settings.enabled && settings.transport == McpTransport::Http {
            let _ = self.start_http_server().await?;
        } else {
            let _ = self.stop_http_server().await?;
        }
        Ok(())
    }

    pub async fn mcp_status(&self) -> McpResult<McpRuntimeStatus> {
        let settings = self.settings_store.load()?;
        let endpoint = self
            .http_runtime
            .lock()
            .await
            .as_ref()
            .map(|server| server.endpoint().to_string());
        self.status_with_endpoint(settings, endpoint)
    }

    pub async fn client_snippets(&self) -> McpResult<McpClientSnippets> {
        let status = self.mcp_status().await?;
        let http_endpoint = status
            .endpoint
            .clone()
            .unwrap_or_else(|| suggested_http_endpoint(&status.settings));

        Ok(McpClientSnippets {
            stdio: serde_json::to_string_pretty(&json!({
                "mcpServers": {
                    "infimount": {
                        "command": "infimount_mcp",
                        "args": ["--transport", "stdio"]
                    }
                }
            }))
            .unwrap_or_default(),
            http: serde_json::to_string_pretty(&json!({
                "mcpServers": {
                    "infimount": {
                        "url": http_endpoint
                    }
                }
            }))
            .unwrap_or_default(),
        })
    }

    async fn stop_http_server_inner(&self) -> McpResult<()> {
        let existing = {
            let mut guard = self.http_runtime.lock().await;
            guard.take()
        };

        if let Some(server) = existing {
            server.stop().await.map_err(map_runtime_io_error)?;
        }
        Ok(())
    }

    fn status_with_endpoint(
        &self,
        settings: McpSettings,
        endpoint: Option<String>,
    ) -> McpResult<McpRuntimeStatus> {
        let endpoint_display = if let Some(endpoint) = &endpoint {
            endpoint.clone()
        } else if settings.transport == McpTransport::Http {
            if settings.port == 0 {
                format!(
                    "Starts on {}:<auto>{}",
                    settings.bind_address, HTTP_ENDPOINT_PATH
                )
            } else {
                suggested_http_endpoint(&settings)
            }
        } else {
            "stdio transport".to_string()
        };

        Ok(McpRuntimeStatus {
            settings,
            running_http: endpoint.is_some(),
            endpoint,
            endpoint_display,
        })
    }
}

fn suggested_http_endpoint(settings: &McpSettings) -> String {
    let port = if settings.port == 0 {
        "<auto>".to_string()
    } else {
        settings.port.to_string()
    };
    format!(
        "http://{}:{}{}",
        settings.bind_address, port, HTTP_ENDPOINT_PATH
    )
}

fn migrate_legacy_sources_if_needed(registry: &StorageRegistry) -> McpResult<()> {
    if registry.path().exists() {
        return Ok(());
    }

    let legacy_sources = config::load_sources().map_err(|err| {
        err_with_details(
            McpErrorCode::ERR_INTERNAL,
            "failed to load legacy storage config",
            json!({ "legacy_error": err.to_string() }),
        )
    })?;

    if legacy_sources.is_empty() {
        return Ok(());
    }

    let storages = legacy_sources
        .into_iter()
        .map(legacy_source_to_storage)
        .collect::<Vec<_>>();
    registry.save_all_atomic(&storages)
}

fn legacy_source_to_storage(source: Source) -> StorageRecord {
    let backend = match source.kind {
        SourceKind::Local => "local",
        SourceKind::S3 => "s3",
        SourceKind::WebDav => "webdav",
        SourceKind::AzureBlob => "azure_blob",
        SourceKind::Gcs => "gcs",
    }
    .to_string();

    let mut config_map = Map::new();
    for (key, value) in source.config.unwrap_or_default() {
        config_map.insert(key, Value::String(value));
    }

    if matches!(backend.as_str(), "local" | "fs") && !source.root.trim().is_empty() {
        config_map
            .entry("root".to_string())
            .or_insert(Value::String(source.root));
    }

    StorageRecord::new(source.name, backend, Value::Object(config_map))
}

pub fn mcp_error_to_core_error(err: McpError) -> CoreError {
    match err.code {
        McpErrorCode::ERR_STORAGE_NOT_FOUND | McpErrorCode::ERR_PATH_NOT_FOUND => CoreError::Io(
            std::io::Error::new(std::io::ErrorKind::NotFound, err.message),
        ),
        McpErrorCode::ERR_PERMISSION_DENIED | McpErrorCode::ERR_STORAGE_READ_ONLY => CoreError::Io(
            std::io::Error::new(std::io::ErrorKind::PermissionDenied, err.message),
        ),
        McpErrorCode::ERR_ALREADY_EXISTS | McpErrorCode::ERR_STORAGE_NAME_CONFLICT => {
            CoreError::Io(std::io::Error::new(
                std::io::ErrorKind::AlreadyExists,
                err.message,
            ))
        }
        _ => CoreError::Config(err.message),
    }
}

fn map_runtime_io_error(err: std::io::Error) -> McpError {
    err_with_details(
        McpErrorCode::ERR_INTERNAL,
        "failed to manage MCP HTTP runtime",
        json!({ "io_error": err.to_string() }),
    )
}
