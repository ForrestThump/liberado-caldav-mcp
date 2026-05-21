mod caldav;
mod models;
mod server;

use server::CaldavServer;
use turbomcp::prelude::*;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt::init();
    let addr = std::env::var("BIND_ADDR").unwrap_or_else(|_| "0.0.0.0:8000".to_string());
    tracing::info!("caldav-mcp listening on {addr}");
    CaldavServer::new()
        .builder()
        .allow_any_origin(true)
        .transport(Transport::http(&addr))
        .serve()
        .await?;
    Ok(())
}
