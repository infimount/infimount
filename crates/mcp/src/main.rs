use infimount_mcp::registry::StorageRegistry;
use infimount_mcp::server::InfimountMcpServer;
use infimount_mcp::tools_fs::FsToolsContext;
use rmcp::ServiceExt;

#[tokio::main]
async fn main() -> Result<(), Box<rmcp::RmcpError>> {
    let transport = std::env::args()
        .skip(1)
        .collect::<Vec<_>>()
        .chunks(2)
        .find_map(|c| {
            if c.len() == 2 && c[0] == "--transport" {
                Some(c[1].clone())
            } else {
                None
            }
        })
        .unwrap_or_else(|| "stdio".to_string());

    if transport != "stdio" {
        eprintln!("unsupported transport: {transport}; only --transport stdio is available");
        std::process::exit(2);
    }

    let ctx = FsToolsContext {
        registry: StorageRegistry::new(None),
    };
    let service = InfimountMcpServer::new(ctx);
    let (stdin, stdout) = rmcp::transport::stdio();
    let running = service
        .serve((stdin, stdout))
        .await
        .map_err(rmcp::RmcpError::from)
        .map_err(Box::new)?;
    let _ = running
        .waiting()
        .await
        .map_err(rmcp::RmcpError::from)
        .map_err(Box::new)?;
    Ok(())
}
